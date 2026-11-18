//! The fixture backend behind `SdkClients::fixtures()`.
//!
//! One claim runs through all of it. Every service must yield the same rows
//! from the same facts the CLI path reads -- asserted per column, because a
//! mis-shaped fixture deserializes into an all-default or half-populated row
//! and a row *count* cannot tell the difference. That failure is not
//! hypothetical for the three services re-shaped last: pointing the Cloud Run
//! stub at the untouched Knative capture answers five services with no name,
//! no region and no URL, and
//! `an_unshaped_knative_capture_fails_on_the_cells_rather_than_the_row_count`
//! is the assertion that sees it.
//!
//! Every service is wired now. There is no refusal walk any more, because
//! there is nothing left to refuse: the `unwired_stub!` macro and `not_wired`
//! went with the last three tranches.
//!
//! Where the same bytes already have a CLI parser, the two backends are also
//! compared row for row. That comparison is what catches a respelled key:
//! proto3 accepts both `messageRetentionDuration` and
//! `message_retention_duration`, so a capture that drifted in either direction
//! still deserializes through the generated model and a native-only assertion
//! that was written from the drifted file never notices.
//!
//! What a read test does NOT prove is that no stub left a method defaulted,
//! and the distinction matters because a generated stub trait's default panics
//! rather than erroring: a defaulted method on a live path aborts the process
//! rather than reporting anything. A read reaches only the methods that read
//! reaches -- every call after a `?` is unreachable while an earlier one
//! errors, which is how compute's `get_operation` stayed missing for a while
//! with the suite green.
//! `every_lifecycle_action_settles_as_success_rather_than_panicking_on_a_defaulted_stub`
//! is the walk that does hold it, for the two services that mutate: it
//! classifies each verb as answered, refused, panicked or hung rather than
//! reading `Err(_)`, because a panic never produces an `Err` to read.
//!
//! It still does not reach everything. A read-back method behind a wait --
//! compute's `get_operation`, Cloud SQL's `operations.get` -- is unreachable
//! while the verbs answer an already-finished operation, which is exactly
//! what they must answer for a demo action not to sleep. Both are implemented
//! anyway, and that property is held statically: by reading each service's
//! call sites *and* the generated builders that call on its behalf.

use std::collections::BTreeMap;

use nuvem::services::sdk::{ClientSource, SdkClients};
use nuvem_domain::gcp::resource::{
    CloudFunctionEnvironment, CloudFunctionState, CloudRunServiceStatus, CloudSqlAvailability,
    CloudSqlStatus, ComputeKeyValue, DiskStatus, EndpointVisibility, FirewallAction,
    FirewallDirection, FirewallTargets, GkeClusterStatus, IamMemberKind, PubSubDeadLetterPolicy,
    PubSubDelivery, PubSubSubscriptionState, ResourceStatus, SecretVersionState, SubscriptionCount,
    VpcRoutingMode, VpcSubnetMode,
};

/// The project the captures were taken from, and one of the two this demo
/// universe contains.
///
/// The fixture clients do not *choose* a document by it -- each answers one
/// capture per call -- but they no longer ignore it either: a listing aimed at
/// a project the demo corpus has never heard of, or at a parent shaped
/// differently from the one its RPC binds, is refused. See
/// `the_stubs_read_the_request_before_they_answer_it` below.
const PROJECT: &str = "dev-project";

const TOPICS_JSON: &str = include_str!("fixtures/pubsub_topics.json");
const SUBSCRIPTIONS_JSON: &str = include_str!("fixtures/pubsub_subscriptions.json");

fn labels(pairs: &[ComputeKeyValue]) -> Vec<(&str, &str)> {
    pairs
        .iter()
        .map(|pair| (pair.key.as_str(), pair.value.as_str()))
        .collect()
}

/// Every cell of every topic row, including the count the second call derives.
///
/// Whole-row assertions per column rather than a length check: proto3 JSON
/// accepts both `messageRetentionDuration` and `message_retention_duration`,
/// and the generated deserializer absorbs a key it does not recognise into
/// `_unknown_fields` instead of failing. A capture that drifted in either
/// direction yields three rows with blank cells, which is exactly the shape a
/// count assertion calls a pass.
#[tokio::test(flavor = "multi_thread")]
async fn the_fixture_topics_carry_every_column_the_table_paints() {
    let rows = nuvem::services::sdk::pubsub::SdkPubSubService::from_fixtures()
        .load_topics(PROJECT)
        .await
        .expect("the fixture client answers");

    assert_eq!(
        rows.iter().map(|row| row.name.as_str()).collect::<Vec<_>>(),
        ["orders", "back-orders", "audit-log"],
        "the capture's three topics, in the capture's order"
    );

    assert_eq!(
        rows[0].full_name, "projects/dev-project/topics/orders",
        "orders: full name"
    );
    assert_eq!(
        rows[0].subscription_count,
        SubscriptionCount::Known(2),
        "orders is named by two of the four captured subscriptions, and the \
         count is the second call's answer rather than the topic's"
    );
    assert_eq!(
        rows[0].message_retention.as_deref(),
        Some("604800s"),
        "orders: retention"
    );
    assert_eq!(
        rows[0].kms_key.as_deref(),
        Some("projects/dev-project/locations/europe-west1/keyRings/nuvem/cryptoKeys/pubsub"),
        "orders: KMS key"
    );
    assert_eq!(
        labels(&rows[0].labels),
        [("env", "prod"), ("team", "platform")],
        "orders: labels, sorted by key"
    );

    assert_eq!(
        rows[1].full_name, "projects/dev-project/topics/back-orders",
        "back-orders: full name"
    );
    assert_eq!(
        rows[1].subscription_count,
        SubscriptionCount::Known(1),
        "back-orders: subscription count"
    );
    assert_eq!(
        rows[1].message_retention.as_deref(),
        Some("86400s"),
        "back-orders: retention"
    );
    assert_eq!(rows[1].kms_key, None, "back-orders: KMS key");
    assert_eq!(
        labels(&rows[1].labels),
        [] as [(&str, &str); 0],
        "back-orders: labels"
    );

    assert_eq!(
        rows[2].full_name, "projects/dev-project/topics/audit-log",
        "audit-log: full name"
    );
    assert_eq!(
        rows[2].subscription_count,
        SubscriptionCount::Known(0),
        "no subscription names audit-log, and the subscriptions call answered, \
         so zero is known rather than unavailable"
    );
    assert_eq!(rows[2].message_retention, None, "audit-log: retention");
    assert_eq!(rows[2].kms_key, None, "audit-log: KMS key");
    assert_eq!(
        labels(&rows[2].labels),
        [] as [(&str, &str); 0],
        "audit-log: labels"
    );
}

/// Every cell of every subscription row.
#[tokio::test(flavor = "multi_thread")]
async fn the_fixture_subscriptions_carry_every_column_the_table_paints() {
    let rows = nuvem::services::sdk::pubsub::SdkPubSubService::from_fixtures()
        .load_subscriptions(PROJECT)
        .await
        .expect("the fixture client answers");

    assert_eq!(
        rows.iter().map(|row| row.name.as_str()).collect::<Vec<_>>(),
        [
            "orders-worker",
            "orders-audit",
            "back-orders-worker",
            "orphaned"
        ]
    );

    assert_eq!(
        rows[0].full_name, "projects/dev-project/subscriptions/orders-worker",
        "orders-worker: full name"
    );
    assert_eq!(
        rows[0].topic, "projects/dev-project/topics/orders",
        "orders-worker: parent topic"
    );
    assert_eq!(
        rows[0].delivery,
        PubSubDelivery::Pull,
        "orders-worker: delivery"
    );
    assert_eq!(
        rows[0].ack_deadline_seconds,
        Some(60),
        "orders-worker: ack deadline"
    );
    assert_eq!(
        rows[0].message_retention.as_deref(),
        Some("604800s"),
        "orders-worker: retention"
    );
    assert_eq!(rows[0].dead_letter, None, "orders-worker: dead letter");
    assert_eq!(
        rows[0].state,
        PubSubSubscriptionState::Active,
        "orders-worker: state"
    );
    assert_eq!(
        labels(&rows[0].labels),
        [("team", "platform")],
        "orders-worker: labels"
    );

    assert_eq!(
        rows[1].topic, "projects/dev-project/topics/orders",
        "orders-audit: parent topic"
    );
    assert_eq!(
        rows[1].delivery,
        PubSubDelivery::Push {
            endpoint: "https://audit.example.com/pubsub".to_string()
        },
        "the endpoint travels with the case that has one"
    );
    assert_eq!(
        rows[1].ack_deadline_seconds,
        Some(10),
        "orders-audit: ack deadline"
    );
    assert_eq!(rows[1].message_retention, None, "orders-audit: retention");
    assert_eq!(
        rows[1].dead_letter,
        Some(PubSubDeadLetterPolicy {
            dead_letter_topic: "projects/dev-project/topics/dead-letters".to_string(),
            max_delivery_attempts: Some(5),
        }),
        "orders-audit: dead letter policy"
    );
    assert_eq!(
        rows[1].state,
        PubSubSubscriptionState::ResourceError,
        "orders-audit: state"
    );
    assert_eq!(
        labels(&rows[1].labels),
        [] as [(&str, &str); 0],
        "orders-audit: labels"
    );

    assert_eq!(
        rows[2].topic, "projects/dev-project/topics/back-orders",
        "back-orders-worker: parent topic"
    );
    assert_eq!(
        rows[2].delivery,
        PubSubDelivery::Pull,
        "an empty pushConfig is not a push endpoint: the client answers a \
         PushConfig whose endpoint is the empty string, and reading its \
         presence as push would label every pull subscription a push one"
    );
    assert_eq!(
        rows[2].ack_deadline_seconds,
        Some(20),
        "back-orders-worker: ack deadline"
    );
    assert_eq!(
        rows[2].state,
        PubSubSubscriptionState::Unspecified,
        "proto3 omits a zero-valued enum, so a missing state IS unspecified"
    );

    assert_eq!(
        rows[3].topic, "_deleted-topic_",
        "a real value the API returns, which matches no topic row"
    );
    assert_eq!(rows[3].delivery, PubSubDelivery::Pull, "orphaned: delivery");
    assert_eq!(
        rows[3].ack_deadline_seconds,
        Some(10),
        "orphaned: ack deadline"
    );
    assert_eq!(
        rows[3].state,
        PubSubSubscriptionState::Active,
        "orphaned: state"
    );
}

/// The fixture client and the CLI parser agree, cell for cell, on the same
/// bytes.
///
/// This is the claim that makes the envelope safe. `envelope` supplies the
/// message wrapper at runtime so there is one copy of each capture, and what
/// has to hold is that wrapping and deserializing through the generated model
/// lands on the same row the hand-written `serde` shape lands on. Where they
/// disagree, demo mode would change what it paints on the day the backend
/// flips -- silently, because both halves would still be "green".
#[tokio::test(flavor = "multi_thread")]
async fn the_fixture_rows_match_the_cli_rows_parsed_from_the_same_bytes() {
    use nuvem::services::resources::pubsub::{
        apply_subscription_counts, parse_subscriptions_json, parse_topics_json,
    };

    let service = nuvem::services::sdk::pubsub::SdkPubSubService::from_fixtures();

    let mut cli_topics = parse_topics_json(TOPICS_JSON).expect("the capture parses");
    let cli_subscriptions =
        parse_subscriptions_json(SUBSCRIPTIONS_JSON).expect("the capture parses");
    apply_subscription_counts(&mut cli_topics, &cli_subscriptions);

    assert_eq!(
        service.load_topics(PROJECT).await.expect("answers"),
        cli_topics,
        "the two backends must paint the same topics table from the same file"
    );
    assert_eq!(
        service.load_subscriptions(PROJECT).await.expect("answers"),
        cli_subscriptions,
        "and the same subscriptions table"
    );
}

/// A cache built by `fixtures()` says so, and one built by `new` does not.
#[test]
fn a_fixture_cache_reports_its_source() {
    assert_eq!(SdkClients::fixtures().source(), ClientSource::Fixtures);
    assert_eq!(
        SdkClients::new(std::sync::Arc::new(
            nuvem::services::demo::DemoCommandRunner::new()
        ))
        .source(),
        ClientSource::Live,
        "the runner does not decide the source: a live cache over the demo \
         runner is still a live cache, which is what the seeders rely on"
    );
}

/// The cache hands out a fixture-backed client, and it is the client the
/// service's own `from_fixtures` builds.
#[tokio::test(flavor = "multi_thread")]
async fn the_fixture_cache_hands_out_the_wired_pubsub_client() {
    let clients = SdkClients::fixtures();

    let service = clients.pubsub("dev").expect("no credentials are needed");
    let rows = service.load_topics(PROJECT).await.expect("answers");

    assert_eq!(
        rows.iter().map(|row| row.name.as_str()).collect::<Vec<_>>(),
        ["orders", "back-orders", "audit-log"],
        "the cache's client is the fixture-backed one, not a live one"
    );
    assert_eq!(
        clients.cached_configurations(),
        ["dev".to_string()],
        "and it is cached per configuration exactly as a live client is"
    );
}

/// Every lifecycle verb on both mutating services settles, and settles as a
/// success.
///
/// # Why this is its own walk
///
/// The per-service tests in this file assert what a read *answers*. This one
/// asserts something weaker and more urgent about the two services that
/// mutate: that the call comes back at all. A generated stub trait gives every method a
/// default of `unimplemented!()`, so a verb whose path reaches a method nobody
/// spelled out does not refuse -- it panics, and in the application that panic
/// is raised on a blocking effect thread and takes the process with it. A
/// refusal is a message in a pane; a panic is a dead nuvem. The two are not
/// neighbouring failure modes and an assertion that only reads `Err(_)` cannot
/// tell them apart, because a panic never produces an `Err` to read.
///
/// So each verb runs in its own task and is classified by how it ended:
/// refused, answered, panicked, or still running when the deadline passed.
///
/// `Panicked` carries the panic's message, which is worth knowing is *generic*:
/// the generated default panics through `gaxi::unimplemented::unimplemented_stub`
/// with one boilerplate string for every method, so the payload says a stub was
/// left defaulted but never which one. What localises it is running each verb
/// on its own -- the failing action names the path, and the reachable methods on
/// that path are then read off the generated builder. Measured, by deleting
/// `InstancesStub::stop`: this walk reported `compute Stop => Panicked(...)`
/// and `Answered` for the other eight, where a single sequential `await`
/// would have unwound on `Stop` and said nothing about `Reset` onwards.
///
/// `Hung` exists because a poller that keeps re-reading an operation is the
/// other way a mutation fails to come back, and a walk that simply `await`ed
/// would hang with it rather than report it. It is the failure the operation
/// each stub answers is shaped to avoid: Compute's poller completes on the
/// first call only for an operation already at `DONE`, and Cloud SQL's
/// `await_operation` returns before its first 500 ms sleep only for one
/// already at `Done`. Measured, by answering `SqlOperationStatus::Running`
/// from `cloudsql::finished`: all four Cloud SQL verbs came back `Hung`, the
/// five compute verbs still `Answered`, and the walk failed in 5.04 s naming
/// `cloud sql Start`. Measured again with only the *started* operation at
/// `Running` and `operations.get` still answering `Done`: every verb
/// `Answered` and the walk went from 0.06 s to 0.53 s, which is the one 500 ms
/// sleep `OPERATION_POLL_INTERVAL_MILLIS` costs an action that has to read
/// its operation back.
///
/// # Why `Refused` is now a failure rather than the expectation
///
/// Both services are wired, so a refusal here means a *read* of them broke:
/// a stub answered an error, or a capture stopped deserializing. Either
/// way the operations log would settle the action at `Failed`, and an
/// operator watching demo mode would see a lifecycle action that does not
/// work. `Answered` is the only passing outcome, and it is a real assertion
/// rather than a weak one: an errored operation reaches
/// `finished_or_error`/`await_operation` as an `Err`, so `Answered` means the
/// operation completed *and* carried no error.
///
/// # Why the variants are not listed here
///
/// `ComputeInstanceAction::ALL` and `CloudSqlInstanceAction::ALL` are generated
/// by `lifecycle_actions!` from each enum's own variant list, so a verb added
/// later joins this walk by existing. A hand-written list would instead be
/// calibrated to the verbs that existed on the day it was written -- which is
/// exactly how compute's `get_operation` came to be missing while every test
/// stayed green: the service never names that method, the generated poller
/// calls it on the service's behalf, and nothing enumerated the reachable set.
#[tokio::test(flavor = "multi_thread")]
async fn every_lifecycle_action_settles_as_success_rather_than_panicking_on_a_defaulted_stub() {
    use nuvem::app::state::resources::{CloudSqlInstanceAction, ComputeInstanceAction};
    use nuvem::services::sdk;

    /// Long enough that a slow machine is not mistaken for a hang, short
    /// enough that a real hang is reported rather than waited on -- and, the
    /// constraint that actually sets the number, short enough that this walk
    /// reports the hang itself instead of being killed mid-report.
    ///
    /// `.config/nextest.toml` terminates any test still running at ten
    /// seconds, on the stated ground that ten seconds in this suite means
    /// hung rather than slow. A deadline above that ceiling makes the `Hung`
    /// branch unreachable under the harness the suite actually runs on:
    /// measured, at the 30 s this started out as, four hung Cloud SQL verbs
    /// produced `TIMEOUT [10.004s]` from nextest and not one word from this
    /// walk about which verb hung or why.
    ///
    /// It is one deadline for the whole walk rather than one per verb, so
    /// that the total stays under the ceiling however many verbs the two
    /// enums come to hold. Each verb still runs in its own task; they are
    /// spawned together and then collected against a single instant.
    const DEADLINE: std::time::Duration = std::time::Duration::from_secs(5);

    /// The zone and instance the verbs are aimed at. The fixture clients
    /// ignore both -- they answer one document per call -- but a lifecycle
    /// request carries them, so the captured names keep the request honest.
    const ZONE: &str = "europe-west1-b";
    const COMPUTE_INSTANCE: &str = "api-dev-1";
    const SQL_INSTANCE: &str = "pg-standalone";

    /// How one verb ended.
    #[derive(Debug)]
    enum Settled {
        /// It answered an error. The string is that error's message.
        Refused(String),
        /// It answered success: the operation completed and carried no error.
        Answered,
        /// It panicked. The string is the panic's message, which for a
        /// defaulted stub method is the generated boilerplate rather than
        /// the method's name -- the verb that panicked is what names the path.
        Panicked(String),
        /// It was still running when `DEADLINE` passed.
        Hung,
    }

    /// Reads a panic payload back as text.
    ///
    /// `unimplemented!` panics with a formatted `String`; `assert!` and a bare
    /// `panic!("literal")` panic with a `&'static str`. Both are checked
    /// because the point of this branch is to *report* the panic, and a
    /// payload that came back as "no message" would leave the failure with
    /// nothing to distinguish it from any other abort.
    fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
        if let Some(message) = payload.downcast_ref::<&'static str>() {
            return (*message).to_string();
        }
        if let Some(message) = payload.downcast_ref::<String>() {
            return message.clone();
        }
        "a panic carrying no message".to_string()
    }

    /// Runs one verb to whichever of the four ends it reaches, giving up at
    /// the walk's shared `deadline`.
    ///
    /// The call runs in its own task so that a panic inside it is caught and
    /// named here rather than unwinding the walk on its first casualty: the
    /// walk's value is the whole table, and a panic on `start` must not hide
    /// what `stop` and `failover` do.
    async fn settled(
        deadline: tokio::time::Instant,
        mut handle: tokio::task::JoinHandle<Result<(), sdk::ApiError>>,
    ) -> Settled {
        match tokio::time::timeout_at(deadline, &mut handle).await {
            Err(_elapsed) => {
                handle.abort();
                Settled::Hung
            }
            Ok(Ok(Ok(()))) => Settled::Answered,
            Ok(Ok(Err(error))) => Settled::Refused(error.to_string()),
            Ok(Err(joined)) if joined.is_panic() => {
                Settled::Panicked(panic_message(joined.into_panic()))
            }
            Ok(Err(joined)) => panic!("the walk's own task was cancelled: {joined}"),
        }
    }

    // Built inside each task rather than shared: `from_fixtures` needs no
    // credentials and cannot fail, so a per-task client costs nothing, and it
    // is what lets the future be `'static` and therefore spawnable.
    //
    // Every verb is spawned before any of them is collected, so one hung verb
    // costs the walk its deadline once rather than once per verb.
    let mut running: Vec<(String, tokio::task::JoinHandle<Result<(), sdk::ApiError>>)> = Vec::new();
    for action in ComputeInstanceAction::ALL {
        running.push((
            format!("compute {action:?}"),
            tokio::spawn(async move {
                sdk::compute::SdkComputeService::from_fixtures()
                    .run_action(PROJECT, ZONE, COMPUTE_INSTANCE, action)
                    .await
            }),
        ));
    }
    for action in CloudSqlInstanceAction::ALL {
        running.push((
            format!("cloud sql {action:?}"),
            tokio::spawn(async move {
                sdk::cloudsql::SdkCloudSqlService::from_fixtures()
                    .run_action(PROJECT, SQL_INSTANCE, action)
                    .await
            }),
        ));
    }

    let deadline = tokio::time::Instant::now() + DEADLINE;
    let mut walked: Vec<(String, Settled)> = Vec::new();
    for (call, handle) in running {
        let outcome = settled(deadline, handle).await;
        walked.push((call, outcome));
    }

    assert_eq!(
        walked.len(),
        ComputeInstanceAction::ALL.len() + CloudSqlInstanceAction::ALL.len(),
        "the walk must cover every variant of both enums"
    );

    for (call, outcome) in &walked {
        match outcome {
            Settled::Answered => {}
            Settled::Refused(message) => panic!(
                "{call} REFUSED. Every service is wired, so a refusal means \
                 a stub answered an error for a method nobody wired, or a \
                 capture stopped deserializing -- either way the operations \
                 log settles this action at Failed. It said: {message}"
            ),
            Settled::Panicked(message) => panic!(
                "{call} PANICKED instead of succeeding. A generated stub trait \
                 defaults every method to `unimplemented!()`, so some method \
                 on {call}'s path is one no stub spells out -- in the \
                 application that aborts the process from a blocking effect \
                 thread. The payload is the generated boilerplate and does \
                 not say which method, so read {call}'s generated builder for \
                 the calls it makes on the service's behalf -- a poller's \
                 `get_operation` is the one a reading of the service alone \
                 misses. It said: {message}"
            ),
            Settled::Hung => panic!(
                "{call} was still running after {DEADLINE:?}. A mutation that \
                 never comes back leaves the operations log waiting forever. \
                 The operation the stub answers must be finished already -- \
                 `DONE` for Compute, `Done` for Cloud SQL -- or the wait \
                 re-reads it until something kills the process."
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// VPC
// ---------------------------------------------------------------------------

const NETWORKS_JSON: &str = include_str!("fixtures/vpc_networks.json");
const FIREWALL_RULES_JSON: &str = include_str!("fixtures/vpc_firewall_rules.json");
const SUBNETS_JSON: &str = include_str!("fixtures/vpc_subnets.json");

/// Every cell of the network row.
///
/// `subnet_count` is the one worth naming: the row shows a count, but the
/// field behind it is a 43-element array of subnetwork URLs. A capture that
/// lost the array still yields a `VpcNetwork` with a name and a routing mode,
/// and only the count says so.
#[tokio::test(flavor = "multi_thread")]
async fn the_fixture_networks_carry_every_column_the_table_paints() {
    let rows = nuvem::services::sdk::vpc::SdkVpcService::from_fixtures()
        .load_networks(PROJECT)
        .await
        .expect("the fixture client answers");

    assert_eq!(
        rows.iter().map(|row| row.name.as_str()).collect::<Vec<_>>(),
        ["default"],
        "the capture's one network"
    );
    assert_eq!(
        rows[0].subnet_mode,
        VpcSubnetMode::Auto,
        "default: subnet mode, from autoCreateSubnetworks"
    );
    assert_eq!(
        rows[0].routing_mode,
        VpcRoutingMode::Regional,
        "default: routing mode, one level down in routingConfig"
    );
    assert_eq!(
        rows[0].subnet_count, 43,
        "default: subnet count, which is the length of the subnetworks array \
         rather than a field of its own"
    );
    assert_eq!(
        rows[0].description.as_deref(),
        Some("Default network for the project"),
        "default: description"
    );
}

/// Every cell of every firewall row.
///
/// Three columns here are derived rather than read, and each has a row in the
/// capture that exercises it: `action` from which of `allowed`/`denied` is
/// non-empty, `targets` from the tags/service-accounts pair, and
/// `source_or_dest` from whichever range list matches the direction.
#[tokio::test(flavor = "multi_thread")]
async fn the_fixture_firewall_rules_carry_every_column_the_table_paints() {
    let rows = nuvem::services::sdk::vpc::SdkVpcService::from_fixtures()
        .load_firewall_rules(PROJECT)
        .await
        .expect("the fixture client answers");

    assert_eq!(
        rows.iter().map(|row| row.name.as_str()).collect::<Vec<_>>(),
        [
            "default-allow-icmp",
            "default-allow-internal",
            "default-allow-rdp",
            "default-allow-ssh",
            "allow-web-to-tagged",
            "deny-all-egress",
            "disabled-sa-scoped-ssh",
        ],
        "the capture's seven rules, in the capture's order"
    );

    assert_eq!(
        rows[0].network, "default",
        "default-allow-icmp: network, read out of the self-link"
    );
    assert_eq!(
        rows[0].direction,
        FirewallDirection::Ingress,
        "default-allow-icmp: direction"
    );
    assert_eq!(
        rows[0].priority,
        Some(65534),
        "default-allow-icmp: priority"
    );
    assert_eq!(
        rows[0].action,
        FirewallAction::Allow,
        "default-allow-icmp: action, from a non-empty allowed list"
    );
    assert_eq!(
        rows[0].targets,
        FirewallTargets::AllInstances,
        "default-allow-icmp: targets"
    );
    assert_eq!(
        rows[0].source_or_dest,
        ["0.0.0.0/0"],
        "default-allow-icmp: source ranges"
    );
    assert!(!rows[0].disabled, "default-allow-icmp: disabled");

    assert_eq!(
        rows[4].targets,
        FirewallTargets::Tags(vec!["web-server".to_string()]),
        "allow-web-to-tagged: the target tags travel with the rule that has them"
    );
    assert_eq!(
        rows[4].source_or_dest,
        ["10.0.0.0/8"],
        "allow-web-to-tagged: source ranges"
    );

    assert_eq!(
        rows[5].direction,
        FirewallDirection::Egress,
        "deny-all-egress: direction"
    );
    assert_eq!(
        rows[5].action,
        FirewallAction::Deny,
        "deny-all-egress: action, from a non-empty denied list"
    );
    assert_eq!(
        rows[5].source_or_dest,
        ["0.0.0.0/0"],
        "an egress rule shows its DESTINATION ranges; reading the source list \
         here would paint an empty cell"
    );

    assert_eq!(
        rows[6].priority,
        Some(900),
        "disabled-sa-scoped-ssh: priority"
    );
    assert_eq!(
        rows[6].targets,
        FirewallTargets::ServiceAccounts(vec![
            "workload@nuvem-fixture-project.iam.gserviceaccount.com".to_string()
        ]),
        "disabled-sa-scoped-ssh: service-account targets"
    );
    assert!(
        rows[6].source_or_dest.is_empty(),
        "disabled-sa-scoped-ssh names no ranges at all"
    );
    assert!(rows[6].disabled, "disabled-sa-scoped-ssh: disabled");
}

/// Each subnet is filed under the region its own row names.
///
/// Walked through the paginator rather than through `load_subnets`, because
/// the scope key is the one thing `load_subnets` cannot show: it binds the key
/// out of every entry and discards it, so a key spelled `europe-west1` instead
/// of `regions/europe-west1`, or spelled as a full self-link, or one key
/// holding all four rows, paints an identical table. The generated item
/// paginator flattens the map without reading a key, so nothing downstream
/// notices either. What such a key breaks is the claim the whole demo backend
/// rests on -- that it answers the shape the live API answers -- and this is
/// the only assertion in the file that can see it.
#[tokio::test(flavor = "multi_thread")]
async fn the_fixture_subnets_are_filed_under_the_region_each_row_names() {
    use google_cloud_gax::paginator::ItemPaginator;

    let mut items = nuvem::services::demo::sdk::vpc::clients()
        .1
        .aggregated_list()
        .set_project(PROJECT)
        .by_item();

    let mut scopes: BTreeMap<String, Vec<String>> = BTreeMap::new();
    while let Some(entry) = items.next().await {
        let (scope, scoped) = entry.expect("the fixture client answers");
        scopes.insert(
            scope,
            scoped
                .subnetworks
                .iter()
                .map(|subnet| subnet.ip_cidr_range.clone().unwrap_or_default())
                .collect(),
        );
    }

    assert_eq!(
        scopes.keys().map(String::as_str).collect::<Vec<_>>(),
        [
            "regions/asia-east1",
            "regions/europe-west1",
            "regions/us-central1",
            "regions/us-west1",
        ],
        "four regions in the capture, so four scopes -- not one scope holding \
         four rows, and not four keys spelled without the `regions/` prefix \
         the API uses"
    );
    // Identified by CIDR rather than by name: all four subnets are called
    // `default`, so a name would not say which row landed in which scope.
    assert_eq!(
        scopes["regions/asia-east1"],
        ["10.140.0.0/20"],
        "asia-east1 holds its own subnet"
    );
    assert_eq!(
        scopes["regions/europe-west1"],
        ["10.132.0.0/20"],
        "europe-west1 holds its own subnet"
    );
    assert_eq!(
        scopes["regions/us-central1"],
        ["10.128.0.0/20"],
        "us-central1 holds its own subnet"
    );
    assert_eq!(
        scopes["regions/us-west1"],
        ["10.138.0.0/20"],
        "us-west1 holds its own subnet"
    );
}

/// Every cell of every subnet row.
///
/// Sorted by region before asserting, because the rows do not arrive in the
/// capture's order: the generated aggregated response carries a `HashMap`, and
/// the paginator hands its scopes back in the map's own iteration order, which
/// differs between runs. Sorting makes the cells assertable and is exactly why
/// the scope keys need the separate assertion above -- collapse every row into
/// one scope and this test still passes.
#[tokio::test(flavor = "multi_thread")]
async fn the_fixture_subnets_carry_every_column_the_table_paints() {
    let mut rows = nuvem::services::sdk::vpc::SdkVpcService::from_fixtures()
        .load_subnets(PROJECT)
        .await
        .expect("the fixture client answers");
    rows.sort_by(|a, b| (&a.region, &a.name).cmp(&(&b.region, &b.name)));

    assert_eq!(
        rows.iter()
            .map(|row| row.region.as_str())
            .collect::<Vec<_>>(),
        ["asia-east1", "europe-west1", "us-central1", "us-west1"],
        "one row per captured region, each surviving the walk -- four rows \
         collapsed into one region would be a table that reads as a project \
         with a single subnet"
    );

    assert_eq!(rows[0].name, "default", "asia-east1: name");
    assert_eq!(rows[0].network, "default", "asia-east1: parent network");
    assert_eq!(
        rows[0].ip_cidr_range, "10.140.0.0/20",
        "asia-east1: primary range"
    );
    assert_eq!(
        rows[0].gateway_address.as_deref(),
        Some("10.140.0.1"),
        "asia-east1: gateway"
    );
    assert!(
        !rows[0].private_google_access,
        "asia-east1: private Google access"
    );

    assert_eq!(rows[1].name, "default", "europe-west1: name");
    assert_eq!(rows[1].network, "default", "europe-west1: parent network");
    assert_eq!(
        rows[1].ip_cidr_range, "10.132.0.0/20",
        "europe-west1: primary range"
    );
    assert_eq!(
        rows[1].gateway_address.as_deref(),
        Some("10.132.0.1"),
        "europe-west1: gateway"
    );
    assert!(
        !rows[1].private_google_access,
        "europe-west1: private Google access"
    );

    assert_eq!(rows[2].name, "default", "us-central1: name");
    assert_eq!(rows[2].network, "default", "us-central1: parent network");
    assert_eq!(
        rows[2].ip_cidr_range, "10.128.0.0/20",
        "us-central1: primary range"
    );
    assert_eq!(
        rows[2].gateway_address.as_deref(),
        Some("10.128.0.1"),
        "us-central1: gateway"
    );
    assert!(
        !rows[2].private_google_access,
        "us-central1: private Google access"
    );

    assert_eq!(rows[3].name, "default", "us-west1: name");
    assert_eq!(rows[3].network, "default", "us-west1: parent network");
    assert_eq!(
        rows[3].ip_cidr_range, "10.138.0.0/20",
        "us-west1: primary range"
    );
    assert_eq!(
        rows[3].gateway_address.as_deref(),
        Some("10.138.0.1"),
        "us-west1: gateway"
    );
    assert!(
        rows[3].private_google_access,
        "us-west1 is the one captured subnet with private Google access on, \
         and the column is a plain bool -- a row that lost the flag paints \
         Off rather than blank"
    );
}

/// The VPC fixture rows and the CLI rows, from the same bytes.
#[tokio::test(flavor = "multi_thread")]
async fn the_fixture_vpc_rows_match_the_cli_rows_parsed_from_the_same_bytes() {
    use nuvem::services::resources::vpc::{
        parse_firewall_rules_json, parse_networks_json, parse_subnets_json,
    };

    let service = nuvem::services::sdk::vpc::SdkVpcService::from_fixtures();

    assert_eq!(
        service.load_networks(PROJECT).await.expect("answers"),
        parse_networks_json(NETWORKS_JSON).expect("the capture parses"),
        "the two backends must paint the same networks table from the same file"
    );
    assert_eq!(
        service.load_firewall_rules(PROJECT).await.expect("answers"),
        parse_firewall_rules_json(FIREWALL_RULES_JSON).expect("the capture parses"),
        "and the same firewall table"
    );

    // Both sides sorted: the CLI keeps the capture's order and the native path
    // cannot, because its rows arrive grouped by scope in a `HashMap`'s
    // iteration order. What has to agree is the set of rows and every cell in
    // them, which is what a common ordering compares.
    let mut native_subnets = service.load_subnets(PROJECT).await.expect("answers");
    let mut cli_subnets = parse_subnets_json(SUBNETS_JSON).expect("the capture parses");
    native_subnets.sort_by(|a, b| (&a.region, &a.name).cmp(&(&b.region, &b.name)));
    cli_subnets.sort_by(|a, b| (&a.region, &a.name).cmp(&(&b.region, &b.name)));
    assert_eq!(
        native_subnets, cli_subnets,
        "and the same subnets table, rebuilt from the scope map rather than          read as an array"
    );
}

// ---------------------------------------------------------------------------
// Compute Engine
// ---------------------------------------------------------------------------

const GCE_INSTANCES_JSON: &str = include_str!("fixtures/gce_instances.json");
const GCE_INSTANCE_DETAIL_JSON: &str = include_str!("fixtures/gce_instance_detail.json");

/// Each instance is filed under the zone its own row names.
///
/// The three respellings this catches -- a bare `europe-west1-b`, a full
/// self-link, and one key holding both rows -- are invisible everywhere else.
/// `load_instances` binds the key out of each entry and discards it, and the
/// generated item paginator flattens the map with `items.into_iter()` without
/// reading a key, so all three paint exactly the table the correct spelling
/// paints. That is what makes this failure mode silent rather than loud, and
/// why the keys are asserted by name here instead of being inferred from rows.
#[tokio::test(flavor = "multi_thread")]
async fn the_fixture_instances_are_filed_under_the_zone_each_row_names() {
    use google_cloud_gax::paginator::ItemPaginator;

    let mut items = nuvem::services::demo::sdk::compute::client()
        .aggregated_list()
        .set_project(PROJECT)
        // The same two flags the service sends. The stub reads them, so a
        // request built here without them is one it refuses -- which is the
        // point of it reading them, and worth paying in a test that walks the
        // paginator by hand rather than through `load_*`.
        .set_include_all_scopes(true)
        .set_return_partial_success(true)
        .by_item();

    let mut scopes: BTreeMap<String, Vec<String>> = BTreeMap::new();
    while let Some(entry) = items.next().await {
        let (scope, scoped) = entry.expect("the fixture client answers");
        scopes.insert(
            scope,
            scoped
                .instances
                .iter()
                .map(|instance| instance.name.clone().unwrap_or_default())
                .collect(),
        );
    }

    assert_eq!(
        scopes.keys().map(String::as_str).collect::<Vec<_>>(),
        ["zones/europe-west1-b", "zones/europe-west1-c"],
        "the capture names two zones, so the aggregated answer carries two \
         scopes, each spelled the way the API spells one"
    );
    assert_eq!(
        scopes["zones/europe-west1-b"],
        ["api-dev-1"],
        "europe-west1-b holds the instance whose own `zone` field names it"
    );
    assert_eq!(
        scopes["zones/europe-west1-c"],
        ["worker-dev-1"],
        "and europe-west1-c holds the other one, rather than both landing in \
         the first zone the capture mentions"
    );
}

/// Every cell of every instance row.
///
/// Sorted by zone first: the rows do not arrive in the capture's order,
/// because the generated aggregated response carries a `HashMap` and the
/// paginator hands its scopes back in that map's iteration order, which
/// differs between runs. Sorting is what makes the cells assertable -- and it
/// is also why the scope keys need their own test, since a collapse of both
/// zones into one key leaves every assertion below untouched.
#[tokio::test(flavor = "multi_thread")]
async fn the_fixture_instances_carry_every_column_the_table_paints() {
    let mut rows = nuvem::services::sdk::compute::SdkComputeService::from_fixtures()
        .load_instances(PROJECT)
        .await
        .expect("the fixture client answers");
    rows.sort_by(|a, b| (&a.zone, &a.name).cmp(&(&b.zone, &b.name)));

    assert_eq!(
        rows.iter().map(|row| row.name.as_str()).collect::<Vec<_>>(),
        ["api-dev-1", "worker-dev-1"],
        "both captured instances survive the walk over the scope map, one \
         from each zone"
    );

    assert_eq!(rows[0].id, "1001", "api-dev-1: id, a number on the wire");
    assert_eq!(rows[0].zone, "europe-west1-b", "api-dev-1: zone");
    assert_eq!(rows[0].status, ResourceStatus::Running, "api-dev-1: status");
    assert_eq!(
        rows[0].machine_type, "e2-medium",
        "api-dev-1: machine type, shortened from the self-link"
    );
    assert_eq!(
        rows[0].internal_ip.as_deref(),
        Some("10.0.0.4"),
        "api-dev-1: internal IP, from the first NIC"
    );
    assert_eq!(
        rows[0].external_ip.as_deref(),
        Some("34.10.10.10"),
        "api-dev-1: external IP, from the first NIC's first access config"
    );

    assert_eq!(rows[1].id, "1002", "worker-dev-1: id");
    assert_eq!(rows[1].zone, "europe-west1-c", "worker-dev-1: zone");
    assert_eq!(
        rows[1].status,
        ResourceStatus::Stopped,
        "worker-dev-1: TERMINATED reads as Stopped on both backends"
    );
    assert_eq!(
        rows[1].machine_type, "e2-small",
        "worker-dev-1: machine type"
    );
    assert_eq!(
        rows[1].internal_ip.as_deref(),
        Some("10.0.0.5"),
        "worker-dev-1: internal IP"
    );
    assert_eq!(
        rows[1].external_ip, None,
        "worker-dev-1 has no access config, so no external IP"
    );
}

/// Every field the instance detail pane paints, from the single-instance
/// capture.
///
/// This is the shape the envelope helper does not cover: `instances.get`
/// answers the resource itself, so the stub deserializes the document
/// directly. A wrapper applied anyway would yield an `Instance` with every
/// field defaulted -- a blank name, a blank zone, `Unknown` status and no
/// disks -- which is exactly what the named cells below refuse.
#[tokio::test(flavor = "multi_thread")]
async fn the_fixture_instance_detail_carries_every_field_the_pane_paints() {
    let detail = nuvem::services::sdk::compute::SdkComputeService::from_fixtures()
        .load_instance_detail(PROJECT, "europe-west1-b", "api-dev-1")
        .await
        .expect("the fixture client answers");

    assert_eq!(detail.name, "api-dev-1", "detail: name");
    assert_eq!(
        detail.zone, "europe-west1-b",
        "detail: zone, the tail of the self-link"
    );
    assert_eq!(detail.status, ResourceStatus::Running, "detail: status");
    assert_eq!(
        detail.machine_type, "e2-medium",
        "detail: machine type, the tail of the self-link"
    );
    assert_eq!(
        detail.creation_timestamp.as_deref(),
        Some("2026-07-03T10:11:12.000-07:00"),
        "detail: creation timestamp, passed through as the capture spells it"
    );

    assert_eq!(
        detail.network_interfaces.len(),
        1,
        "the capture carries one NIC"
    );
    let nic = &detail.network_interfaces[0];
    assert_eq!(nic.name.as_deref(), Some("nic0"), "nic0: name");
    assert_eq!(
        nic.network.as_deref(),
        Some("default"),
        "nic0: network, shortened from the self-link"
    );
    assert_eq!(
        nic.subnetwork.as_deref(),
        Some("default"),
        "nic0: subnetwork, shortened from the self-link"
    );
    assert_eq!(
        nic.subnetwork_region.as_deref(),
        Some("europe-west1"),
        "nic0: the region read out of the middle of the subnetwork URL, not \
         off a field of its own"
    );
    assert_eq!(
        nic.internal_ip.as_deref(),
        Some("10.0.0.4"),
        "nic0: internal IP"
    );
    assert_eq!(
        nic.external_ip.as_deref(),
        Some("34.10.10.10"),
        "nic0: external IP, from the first access config"
    );

    assert_eq!(
        detail
            .disks
            .iter()
            .map(|disk| disk.name.as_deref().unwrap_or_default())
            .collect::<Vec<_>>(),
        ["api-dev-1", "data-disk"],
        "detail: both attached disks, in the capture's order"
    );
    assert_eq!(
        detail.disks[0].source.as_deref(),
        Some("api-dev-1"),
        "the boot disk's source, shortened from the self-link"
    );
    assert_eq!(
        (detail.disks[0].boot, detail.disks[0].auto_delete),
        (true, true),
        "the boot disk is the boot disk and auto-deletes"
    );
    assert_eq!(
        detail.disks[0].disk_type.as_deref(),
        Some("PERSISTENT"),
        "disk type is an enum on the wire here and a name in the pane"
    );
    assert_eq!(
        detail.disks[0].size_gb.as_deref(),
        Some("20"),
        "disk size: a quoted int64 on the wire, a string in the pane"
    );
    assert_eq!(
        (detail.disks[1].boot, detail.disks[1].auto_delete),
        (false, false),
        "the data disk neither boots nor auto-deletes, so `false` here is the \
         capture's own value rather than a default standing in for one"
    );
    assert_eq!(
        detail.disks[1].size_gb.as_deref(),
        Some("100"),
        "the data disk's size"
    );

    assert_eq!(
        detail
            .metadata
            .iter()
            .map(|item| (item.key.as_str(), item.value.as_str()))
            .collect::<Vec<_>>(),
        [("startup-script", "echo hello"), ("owner", "platform")],
        "detail: metadata, a repeated field so the capture's order survives"
    );
    // Sorted before comparing: the generated model holds `labels` in a
    // `HashMap` and the mapper collects it in iteration order, so the pane's
    // label order differs between runs of the process. The CLI parser has the
    // same `HashMap` behind it. Reported here rather than fixed in this slice,
    // exactly as for GKE's cluster detail.
    let mut detail_labels = labels(&detail.labels);
    detail_labels.sort_unstable();
    assert_eq!(
        detail_labels,
        [("app", "api"), ("env", "dev")],
        "detail: labels"
    );

    assert_eq!(
        detail.tags,
        [] as [String; 0],
        "the capture carries no tags, and an absent block means none rather \
         than unknown"
    );
    assert_eq!(
        detail.service_accounts,
        [] as [String; 0],
        "and no service accounts"
    );
    assert!(
        detail.raw_json.contains("api-dev-1"),
        "the JSON tab is rendered from the client's own model, so it must at \
         least carry the instance it describes; said: {}",
        detail.raw_json
    );
}

/// The Compute Engine fixture rows and the CLI rows, from the same bytes.
#[tokio::test(flavor = "multi_thread")]
async fn the_fixture_instance_rows_match_the_cli_rows_parsed_from_the_same_bytes() {
    use nuvem::services::resources::compute::{parse_instance_detail_json, parse_instances_json};

    let service = nuvem::services::sdk::compute::SdkComputeService::from_fixtures();
    let mut native = service.load_instances(PROJECT).await.expect("answers");
    let mut cli = parse_instances_json(GCE_INSTANCES_JSON).expect("the capture parses");
    // Sorted on both sides: only the CLI path can keep the capture's order,
    // because only it reads an array. What has to agree is the rows and their
    // cells.
    native.sort_by(|a, b| (&a.zone, &a.name).cmp(&(&b.zone, &b.name)));
    cli.sort_by(|a, b| (&a.zone, &a.name).cmp(&(&b.zone, &b.name)));

    assert_eq!(
        native, cli,
        "the two backends must paint the same instances table from the same \
         file, one reading a flat array and one walking the scope map rebuilt \
         from it"
    );

    // And the detail pane, field for field. `raw_json` is compared separately
    // and loosely for the reason GKE's is: the CLI pane pretty-prints
    // gcloud's document and the native pane serializes the generated model,
    // so the two texts are free to differ while every painted field must not.
    let mut native = service
        .load_instance_detail(PROJECT, "europe-west1-b", "api-dev-1")
        .await
        .expect("answers");
    let mut cli = parse_instance_detail_json(GCE_INSTANCE_DETAIL_JSON).expect("the capture parses");
    assert!(
        !native.raw_json.is_empty(),
        "the native pane has a JSON tab"
    );
    assert!(!cli.raw_json.is_empty(), "so does the CLI pane");
    native.raw_json.clear();
    cli.raw_json.clear();
    // Both sides read `labels` out of a `HashMap`, so both orders are whatever
    // that run's hasher produced; sorted here so the comparison measures
    // content rather than the hasher.
    native.labels.sort_by(|a, b| a.key.cmp(&b.key));
    cli.labels.sort_by(|a, b| a.key.cmp(&b.key));
    assert_eq!(native, cli, "and the same instance detail, field for field");
}

// ---------------------------------------------------------------------------
// Cloud SQL
// ---------------------------------------------------------------------------

const CLOUDSQL_INSTANCES_JSON: &str = include_str!("fixtures/cloudsql_instances.json");
const CLOUDSQL_INSTANCE_DETAIL_JSON: &str = include_str!("fixtures/cloudsql_instance_detail.json");
const CLOUDSQL_USERS_JSON: &str = include_str!("fixtures/cloudsql_users.json");

/// Every cell of every instance row.
///
/// CONNECTIVITY is the column to watch: like GKE's endpoint visibility it is
/// not a field but a verdict, here over three inputs -- whether a private
/// network is configured, whether IPv4 is enabled, and whether any assigned
/// address is `PRIMARY`. The capture carries an instance that is public, one
/// that is private, and one with no addresses at all.
#[tokio::test(flavor = "multi_thread")]
async fn the_fixture_cloudsql_instances_carry_every_column_the_table_paints() {
    let rows = nuvem::services::sdk::cloudsql::SdkCloudSqlService::from_fixtures()
        .load_instances(PROJECT)
        .await
        .expect("the fixture client answers");

    assert_eq!(
        rows.iter().map(|row| row.name.as_str()).collect::<Vec<_>>(),
        [
            "pg-standalone",
            "mysql-ha",
            "pg-replica",
            "pg-stopped",
            "sqlserver-maint"
        ],
        "the capture's five instances, in the capture's order -- this listing \
         is a repeated field rather than a scope map, so the order survives"
    );

    assert_eq!(
        rows[0].database_version, "POSTGRES_15",
        "pg-standalone: the raw version, an enum on the wire"
    );
    assert_eq!(
        rows[0].engine, "PostgreSQL",
        "pg-standalone: the engine, normalized from that version"
    );
    assert_eq!(rows[0].region, "us-central1", "pg-standalone: region");
    assert_eq!(rows[0].tier, "db-custom-2-7680", "pg-standalone: tier");
    assert_eq!(
        rows[0].availability,
        CloudSqlAvailability::Zonal,
        "pg-standalone: availability"
    );
    assert_eq!(
        rows[0].status,
        CloudSqlStatus::Runnable,
        "pg-standalone: status"
    );
    assert_eq!(
        rows[0].connectivity,
        EndpointVisibility::Public,
        "pg-standalone has IPv4 on, a PRIMARY address and no private network"
    );

    assert_eq!(rows[1].database_version, "MYSQL_8_0", "mysql-ha: version");
    assert_eq!(rows[1].engine, "MySQL", "mysql-ha: engine");
    assert_eq!(rows[1].region, "us-east1", "mysql-ha: region");
    assert_eq!(rows[1].tier, "db-n1-standard-4", "mysql-ha: tier");
    assert_eq!(
        rows[1].availability,
        CloudSqlAvailability::Regional,
        "mysql-ha: availability"
    );
    assert_eq!(rows[1].status, CloudSqlStatus::Runnable, "mysql-ha: status");
    assert_eq!(
        rows[1].connectivity,
        EndpointVisibility::Private,
        "mysql-ha names a private network and has IPv4 off. `privateNetwork` \
         is a bare string in the generated model, so `configured` is \
         `not empty` here where the CLI parser reads a present key"
    );

    assert_eq!(rows[2].name, "pg-replica", "pg-replica: name");
    assert_eq!(rows[2].tier, "db-custom-2-7680", "pg-replica: tier");
    assert_eq!(
        rows[2].connectivity,
        EndpointVisibility::Public,
        "pg-replica carries no addresses at all, so the verdict rests on \
         ipv4Enabled alone"
    );

    assert_eq!(
        rows[3].database_version, "POSTGRES_14",
        "pg-stopped: version"
    );
    assert_eq!(rows[3].region, "europe-west1", "pg-stopped: region");
    assert_eq!(
        rows[3].status,
        CloudSqlStatus::Stopped,
        "pg-stopped: STOPPED reads as Stopped on both backends"
    );

    assert_eq!(
        rows[4].database_version, "SQLSERVER_2019_STANDARD",
        "sqlserver-maint: version"
    );
    assert_eq!(
        rows[4].engine, "SQL Server",
        "sqlserver-maint: engine, normalized from a third prefix"
    );
    assert_eq!(
        rows[4].status,
        CloudSqlStatus::Maintenance,
        "sqlserver-maint: status"
    );
    assert_eq!(
        rows[4].availability,
        CloudSqlAvailability::Regional,
        "sqlserver-maint: availability"
    );
}

/// Every field the instance detail pane paints, from the single-instance
/// capture.
///
/// `instances.get` answers the resource itself, so this is the shape the
/// envelope helper does not cover and the stub deserializes the document
/// directly. A wrapper applied anyway would yield a `DatabaseInstance` with
/// every field defaulted -- a blank name, `Unknown` status, no backup block --
/// which is what the named cells below refuse.
#[tokio::test(flavor = "multi_thread")]
async fn the_fixture_cloudsql_instance_detail_carries_every_field_the_pane_paints() {
    let detail = nuvem::services::sdk::cloudsql::SdkCloudSqlService::from_fixtures()
        .load_instance_detail(PROJECT, "mysql-ha")
        .await
        .expect("the fixture client answers");

    assert_eq!(detail.name, "mysql-ha", "detail: name");
    assert_eq!(detail.database_version, "MYSQL_8_0", "detail: version");
    assert_eq!(detail.engine, "MySQL", "detail: engine");
    assert_eq!(detail.region, "us-east1", "detail: region");
    assert_eq!(detail.tier, "db-n1-standard-4", "detail: tier");
    assert_eq!(
        detail.availability,
        CloudSqlAvailability::Regional,
        "detail: availability"
    );
    assert_eq!(detail.status, CloudSqlStatus::Runnable, "detail: status");
    assert_eq!(
        detail.activation_policy.as_deref(),
        Some("ALWAYS"),
        "detail: activation policy, an enum on the wire and its name in the pane"
    );
    assert_eq!(
        detail.creation_timestamp, None,
        "the capture carries no createTime, and a blank cell is the honest \
         rendering of a field it does not have"
    );
    assert_eq!(
        detail.connection_name.as_deref(),
        Some("proj:us-east1:mysql-ha"),
        "detail: connection name"
    );
    assert_eq!(
        detail.public_ip.as_deref(),
        Some("34.10.0.9"),
        "detail: the PRIMARY address"
    );
    assert_eq!(
        detail.private_ip.as_deref(),
        Some("10.1.2.3"),
        "detail: the PRIVATE address, picked by type rather than by position"
    );
    assert!(detail.require_ssl, "detail: SSL required");
    assert_eq!(
        detail.private_network.as_deref(),
        Some("projects/p/global/networks/default"),
        "detail: private network"
    );
    assert_eq!(
        detail.maintenance_version.as_deref(),
        Some("MYSQL_8_0_36.R20240101.00_01"),
        "detail: maintenance version"
    );

    let window = detail
        .maintenance_window
        .as_ref()
        .expect("the capture carries a maintenance window");
    assert_eq!(window.day, Some(7), "maintenance window: day");
    assert_eq!(window.hour, Some(3), "maintenance window: hour");
    assert_eq!(
        window.update_track.as_deref(),
        Some("stable"),
        "maintenance window: update track, whose enum name is lower-case on \
         this API and must survive as the capture spells it"
    );

    assert!(detail.backup.enabled, "backup: enabled");
    assert_eq!(
        detail.backup.start_time.as_deref(),
        Some("03:00"),
        "backup: start time"
    );
    assert_eq!(
        detail.backup.location.as_deref(),
        Some("us"),
        "backup: location"
    );
    assert!(
        detail.backup.point_in_time_recovery,
        "backup: point-in-time recovery"
    );
    assert_eq!(
        detail.backup.transaction_log_retention_days,
        Some(7),
        "backup: transaction log retention"
    );

    assert_eq!(
        detail.replicas,
        ["mysql-ha-replica"],
        "detail: replica names"
    );
    assert_eq!(
        detail.master_instance, None,
        "mysql-ha is nobody's replica, and an empty string on the wire must \
         read as no master rather than as a master named \"\""
    );
    assert_eq!(
        detail.users,
        [] as [nuvem_domain::gcp::resource::CloudSqlUser; 0],
        "the Users tab fetches separately, so the detail call leaves this \
         empty on both backends rather than paying a second call per drill-in"
    );

    // Sorted before comparing: `user_labels` is a `HashMap` in the generated
    // model and in the CLI parser's raw struct alike, so the pane's label
    // order differs between runs of the process. Reported here rather than
    // fixed in this slice, as for GKE's cluster detail.
    let mut detail_labels = labels(&detail.labels);
    detail_labels.sort_unstable();
    assert_eq!(
        detail_labels,
        [("env", "prod"), ("team", "platform")],
        "detail: user labels"
    );

    assert!(
        detail.raw_json.contains("mysql-ha"),
        "the JSON tab is rendered from the client's own model, so it must at \
         least carry the instance it describes; said: {}",
        detail.raw_json
    );
    assert!(
        !detail.raw_json.contains("SUPERSECRET")
            && !detail.raw_json.contains("ARRAYSECRETKEY")
            && !detail.raw_json.contains("BEGIN CERTIFICATE"),
        "the capture carries a root password, a client key and two PEM \
         blocks, and the JSON tab must not: the native pane redacts through \
         the same walk the CLI pane does. Said: {}",
        detail.raw_json
    );

    // The absence above is necessary and not sufficient: it reads the same way
    // whether the material was redacted or the whole `sslCerts` array was
    // dropped on the floor, and `DatabaseInstance` has no `sslCerts` field --
    // it is absorbed into the model's private `_unknown_fields` map. The
    // generated serializer writes that map back out, so the array does reach
    // `redact_secrets` and is walked like any other. Named cells, because a
    // dropped array and a redacted one are opposite conclusions.
    let rendered: serde_json::Value =
        serde_json::from_str(&detail.raw_json).expect("the JSON tab renders JSON");
    assert_eq!(
        rendered["sslCerts"][0]["commonName"],
        serde_json::json!("client-cert-1"),
        "the client certificate survives into the pane, unmodelled field or \
         not: {}",
        detail.raw_json
    );
    assert_eq!(
        [
            &rendered["sslCerts"][0]["cert"],
            &rendered["sslCerts"][0]["clientKey"],
        ],
        [
            &serde_json::json!("<redacted>"),
            &serde_json::json!("<redacted>"),
        ],
        "and both of its secret-bearing cells read `<redacted>` rather than \
         being absent: the redaction walk reaches a field the model never \
         modelled. {}",
        detail.raw_json
    );
}

/// Every cell of every user row.
#[tokio::test(flavor = "multi_thread")]
async fn the_fixture_cloudsql_users_carry_every_column_the_tab_paints() {
    let rows = nuvem::services::sdk::cloudsql::SdkCloudSqlService::from_fixtures()
        .load_users(PROJECT, "mysql-ha")
        .await
        .expect("the fixture client answers");

    assert_eq!(
        rows.iter().map(|row| row.name.as_str()).collect::<Vec<_>>(),
        ["postgres", "app"],
        "the capture's two users, in the capture's order"
    );
    assert_eq!(
        rows[0].host, None,
        "postgres is scoped to no host: the capture spells that as an empty \
         string, and `non_empty` turns it into a blank cell"
    );
    assert_eq!(
        rows[1].host.as_deref(),
        Some("%"),
        "app: host, MySQL's any-host wildcard"
    );
    assert_eq!(
        rows[0].kind.as_deref(),
        Some("BUILT_IN"),
        "postgres: TYPE, read off the capture's `type` key. `map_user` read \
         the generated model's `kind` instead -- a different field, documented \
         as the constant `sql#user` -- so this column painted that constant \
         against the live API and a blank cell against this capture, which \
         states no `kind`. The fix was in the mapper, not in the capture"
    );
    assert_eq!(
        rows[1].kind.as_deref(),
        Some("BUILT_IN"),
        "app: TYPE, from its own `type` key"
    );
}

/// The Cloud SQL fixture rows and the CLI rows, from the same bytes.
///
/// `raw_json` is compared separately and loosely, as GKE's is: the CLI pane
/// pretty-prints gcloud's document and the native pane serializes the
/// generated model, which spells every enum as its number where the capture
/// spelled it as a name -- `"state": 1` against `"state": "RUNNABLE"`. The two
/// texts differ by construction; every painted field must not.
#[tokio::test(flavor = "multi_thread")]
async fn the_fixture_cloudsql_rows_match_the_cli_rows_parsed_from_the_same_bytes() {
    use nuvem::services::resources::cloudsql::{
        parse_instance_detail_json, parse_instances_json, parse_users_json,
    };

    let service = nuvem::services::sdk::cloudsql::SdkCloudSqlService::from_fixtures();

    let native = service.load_instances(PROJECT).await.expect("answers");
    let cli = parse_instances_json(CLOUDSQL_INSTANCES_JSON).expect("the capture parses");
    assert_eq!(
        native, cli,
        "the two backends must paint the same instances table from the same \
         file, one reading a flat array and one reading it back out of the \
         `items` field the RPC wraps it in"
    );

    let mut native = service
        .load_instance_detail(PROJECT, "mysql-ha")
        .await
        .expect("answers");
    let mut cli =
        parse_instance_detail_json(CLOUDSQL_INSTANCE_DETAIL_JSON).expect("the capture parses");
    assert!(
        !native.raw_json.is_empty(),
        "the native pane has a JSON tab"
    );
    assert!(!cli.raw_json.is_empty(), "so does the CLI pane");
    native.raw_json.clear();
    cli.raw_json.clear();
    // Both sides read `userLabels` out of a `HashMap`; sorted so the
    // comparison measures content rather than the hasher.
    native.labels.sort_by(|a, b| a.key.cmp(&b.key));
    cli.labels.sort_by(|a, b| a.key.cmp(&b.key));
    assert_eq!(native, cli, "and the same instance detail, field for field");

    let native = service
        .load_users(PROJECT, "mysql-ha")
        .await
        .expect("answers");
    let cli = parse_users_json(CLOUDSQL_USERS_JSON).expect("the capture parses");
    assert_eq!(
        native
            .iter()
            .map(|user| user.name.as_str())
            .collect::<Vec<_>>(),
        cli.iter()
            .map(|user| user.name.as_str())
            .collect::<Vec<_>>(),
        "the same users, in the same order"
    );
    assert_eq!(
        (native[1].host.as_deref(), cli[1].host.as_deref()),
        (Some("%"), Some("%")),
        "a host that is actually set agrees on both backends"
    );
    assert_eq!(
        (native[0].host.as_deref(), cli[0].host.as_deref()),
        (None, Some("")),
        "DIVERGENCE: `host` is a bare string in the generated model, so an \
         empty host arrives as \"\" and `non_empty` blanks it, where the CLI \
         parser carries Some(\"\") through. Both render as an empty cell \
         today; they are not the same value, and a renderer that ever \
         distinguished present-but-empty from absent would split them"
    );
    assert_eq!(
        (native[0].kind.as_deref(), cli[0].kind.as_deref()),
        (Some("BUILT_IN"), Some("BUILT_IN")),
        "CLOSED: the capture states the user type as `type`. \
         `parse_users_json` reads it; `map_user` read the model's `kind` \
         instead, which is the constant `sql#user` on the live API and absent \
         here -- so the native TYPE column was blank where the CLI column said \
         BUILT_IN. It reads `r#type` now, and both columns say the same word"
    );
}

// ---------------------------------------------------------------------------
// Persistent disks
// ---------------------------------------------------------------------------

const DISKS_JSON: &str = include_str!("fixtures/persistent_disks.json");

/// Each disk is filed under the zone or region its own row names.
///
/// Disks are where both kinds of scope appear at once, and where the third
/// kind does too: the capture carries a row with neither a `zone` nor a
/// `region`, which is filed under `global` rather than dropped. Dropping it
/// would paint five rows where the CLI backend paints six, and a row count is
/// the one thing a table's reader has no way to check.
#[tokio::test(flavor = "multi_thread")]
async fn the_fixture_disks_are_filed_under_the_scope_each_row_names() {
    use google_cloud_gax::paginator::ItemPaginator;

    let mut items = nuvem::services::demo::sdk::disks::client()
        .aggregated_list()
        .set_project(PROJECT)
        // The same two flags the service sends. The stub reads them, so a
        // request built here without them is one it refuses -- which is the
        // point of it reading them, and worth paying in a test that walks the
        // paginator by hand rather than through `load_*`.
        .set_include_all_scopes(true)
        .set_return_partial_success(true)
        .by_item();

    let mut scopes: BTreeMap<String, Vec<String>> = BTreeMap::new();
    while let Some(entry) = items.next().await {
        let (scope, scoped) = entry.expect("the fixture client answers");
        scopes.insert(
            scope,
            scoped
                .disks
                .iter()
                .map(|disk| disk.name.clone().unwrap_or_default())
                .collect(),
        );
    }

    assert_eq!(
        scopes.keys().map(String::as_str).collect::<Vec<_>>(),
        [
            "global",
            "regions/europe-west1",
            "zones/europe-west1-b",
            "zones/europe-west1-c",
        ],
        "two zonal scopes, one regional, and the scope-less row's own -- each \
         spelled the way the API spells it, prefix included"
    );
    assert_eq!(
        scopes["zones/europe-west1-b"],
        ["gke-node-a", "detached-legacy"],
        "europe-west1-b holds its two disks, in the capture's order"
    );
    assert_eq!(
        scopes["zones/europe-west1-c"],
        ["being-built", "broken-disk"],
        "and europe-west1-c its two"
    );
    assert_eq!(
        scopes["regions/europe-west1"],
        ["regional-share"],
        "the regional disk is filed under `regions/`, not under the zone \
         whose name its region is a prefix of"
    );
    assert_eq!(
        scopes["global"],
        ["bare-row"],
        "a row with neither field still reaches the walk"
    );
}

/// Every cell of every disk row.
///
/// Sorted by location then name, for the same reason the instances are: the
/// scopes arrive in a `HashMap`'s iteration order.
#[tokio::test(flavor = "multi_thread")]
async fn the_fixture_disks_carry_every_column_the_table_paints() {
    let mut rows = nuvem::services::sdk::disks::SdkDisksService::from_fixtures()
        .load_disks(PROJECT)
        .await
        .expect("the fixture client answers");
    rows.sort_by(|a, b| (&a.location, &a.name).cmp(&(&b.location, &b.name)));

    assert_eq!(
        rows.iter().map(|row| row.name.as_str()).collect::<Vec<_>>(),
        [
            "bare-row",
            "regional-share",
            "detached-legacy",
            "gke-node-a",
            "being-built",
            "broken-disk",
        ],
        "every captured disk survives the walk, from all four scopes"
    );

    assert_eq!(rows[0].location, "", "bare-row: no zone and no region");
    assert_eq!(
        rows[0].status,
        DiskStatus::Unrecognized(String::new()),
        "bare-row: an absent status is unreadable, not a guess at Ready"
    );
    assert_eq!(rows[0].size_gb, 0, "bare-row: size");
    assert_eq!(rows[0].disk_type, "", "bare-row: type");
    assert_eq!(rows[0].users, [] as [String; 0], "bare-row: users");
    assert_eq!(labels(&rows[0].labels), [] as [(&str, &str); 0]);

    assert_eq!(
        rows[1].location, "europe-west1",
        "regional-share: the region, where a zonal disk carries a zone"
    );
    assert_eq!(rows[1].status, DiskStatus::Ready, "regional-share: status");
    assert_eq!(rows[1].size_gb, 200, "regional-share: size");
    assert_eq!(
        rows[1].disk_type, "pd-balanced",
        "regional-share: type, shortened from the self-link"
    );
    assert_eq!(
        rows[1].users,
        ["db-primary", "db-standby"],
        "regional-share: both attached instances, by short name"
    );
    assert_eq!(labels(&rows[1].labels), [] as [(&str, &str); 0]);

    assert_eq!(rows[2].location, "europe-west1-b", "detached-legacy: zone");
    assert_eq!(rows[2].status, DiskStatus::Ready, "detached-legacy: status");
    assert_eq!(rows[2].size_gb, 20, "detached-legacy: size");
    assert_eq!(rows[2].disk_type, "pd-ssd", "detached-legacy: type");
    assert_eq!(
        rows[2].users,
        [] as [String; 0],
        "detached-legacy is attached to nothing, which is what makes it an \
         orphan"
    );
    assert_eq!(labels(&rows[2].labels), [] as [(&str, &str); 0]);

    assert_eq!(rows[3].location, "europe-west1-b", "gke-node-a: zone");
    assert_eq!(rows[3].status, DiskStatus::Ready, "gke-node-a: status");
    assert_eq!(
        rows[3].size_gb, 100,
        "gke-node-a: size, a decimal string on the wire"
    );
    assert_eq!(rows[3].disk_type, "pd-balanced", "gke-node-a: type");
    assert_eq!(rows[3].users, ["gke-node-a"], "gke-node-a: users");
    assert_eq!(
        labels(&rows[3].labels),
        [("env", "prod"), ("team", "platform")],
        "gke-node-a: labels, sorted by key"
    );

    assert_eq!(rows[4].location, "europe-west1-c", "being-built: zone");
    assert_eq!(
        rows[4].status,
        DiskStatus::Creating,
        "being-built: CREATING, which keeps it off the orphan badge"
    );
    assert_eq!(rows[4].size_gb, 50, "being-built: size");
    assert_eq!(rows[4].disk_type, "pd-ssd", "being-built: type");
    assert_eq!(rows[4].users, [] as [String; 0], "being-built: users");
    assert_eq!(labels(&rows[4].labels), [] as [(&str, &str); 0]);

    assert_eq!(rows[5].location, "europe-west1-c", "broken-disk: zone");
    assert_eq!(rows[5].status, DiskStatus::Failed, "broken-disk: status");
    assert_eq!(rows[5].size_gb, 9, "broken-disk: size");
    assert_eq!(rows[5].disk_type, "pd-standard", "broken-disk: type");
    assert_eq!(rows[5].users, [] as [String; 0], "broken-disk: users");
    assert_eq!(labels(&rows[5].labels), [] as [(&str, &str); 0]);
}

/// The persistent disk fixture rows and the CLI rows, from the same bytes.
#[tokio::test(flavor = "multi_thread")]
async fn the_fixture_disk_rows_match_the_cli_rows_parsed_from_the_same_bytes() {
    use nuvem::services::resources::disks::parse_disks_json;

    let mut native = nuvem::services::sdk::disks::SdkDisksService::from_fixtures()
        .load_disks(PROJECT)
        .await
        .expect("answers");
    let mut cli = parse_disks_json(DISKS_JSON).expect("the capture parses");
    native.sort_by(|a, b| (&a.location, &a.name).cmp(&(&b.location, &b.name)));
    cli.sort_by(|a, b| (&a.location, &a.name).cmp(&(&b.location, &b.name)));

    assert_eq!(
        native, cli,
        "the two backends must paint the same disks table from the same file \
         -- including the scope-less row, which the native path only keeps \
         because it files it under `global` rather than dropping it"
    );
}

// ---------------------------------------------------------------------------
// GKE
// ---------------------------------------------------------------------------

const GKE_CLUSTERS_JSON: &str = include_str!("fixtures/gke_clusters.json");
const GKE_CLUSTER_DETAIL_JSON: &str = include_str!("fixtures/gke_cluster_detail.json");

/// Every cell of every cluster row.
///
/// `endpoint_visibility` is the column to watch: it is not a field at all but a
/// verdict over an endpoint string and a private-cluster flag that lives two
/// levels down, and the capture carries one cluster for each of its three
/// answers.
#[tokio::test(flavor = "multi_thread")]
async fn the_fixture_clusters_carry_every_column_the_table_paints() {
    let rows = nuvem::services::sdk::gke::SdkGkeService::from_fixtures()
        .load_clusters(PROJECT)
        .await
        .expect("the fixture client answers");

    assert_eq!(
        rows.iter().map(|row| row.name.as_str()).collect::<Vec<_>>(),
        ["platform-dev", "private-prod", "legacy-unknown"],
        "the capture's three clusters, in the capture's order"
    );

    assert_eq!(rows[0].location, "europe-west1", "platform-dev: location");
    assert_eq!(
        rows[0].status,
        GkeClusterStatus::Running,
        "platform-dev: status"
    );
    assert_eq!(
        rows[0].current_master_version.as_deref(),
        Some("1.31.5-gke.1023000"),
        "platform-dev: master version"
    );
    assert_eq!(
        rows[0].current_node_count,
        Some(6),
        "platform-dev: node count"
    );
    assert_eq!(
        rows[0].endpoint_visibility,
        EndpointVisibility::Public,
        "platform-dev has an endpoint and is not private"
    );

    assert_eq!(
        rows[1].location, "europe-west1-b",
        "private-prod reports where it lives in the deprecated `zone` field \
         rather than in `location`, and the REGION cell has to show it either \
         way -- reading `location` alone painted this blank"
    );
    assert_eq!(
        rows[1].status,
        GkeClusterStatus::Reconciling,
        "private-prod: status"
    );
    assert_eq!(
        rows[1].current_master_version.as_deref(),
        Some("1.30.9-gke.1127000"),
        "private-prod: master version"
    );
    assert_eq!(
        rows[1].current_node_count,
        Some(3),
        "private-prod: node count"
    );
    assert_eq!(
        rows[1].endpoint_visibility,
        EndpointVisibility::Private,
        "private-prod: visibility"
    );

    assert_eq!(rows[2].location, "us-central1", "legacy-unknown: location");
    assert_eq!(
        rows[2].status,
        GkeClusterStatus::Unknown("MAINTENANCE_WINDOW".to_string()),
        "a status outside the enum reads as itself rather than as a guess"
    );
    assert_eq!(
        rows[2].current_master_version, None,
        "legacy-unknown: no master version"
    );
    assert_eq!(
        rows[2].current_node_count, None,
        "legacy-unknown reports no currentNodeCount; proto3 deserializes the \
         omitted key to 0, and passing that through would claim a cluster \
         with zero nodes"
    );
    assert_eq!(
        rows[2].endpoint_visibility,
        EndpointVisibility::Unknown,
        "no endpoint and no private flag is unknown, not public"
    );
}

/// Every field the cluster detail pane paints, from the single-cluster capture.
///
/// This is the shape the envelope helper does not cover: `clusters.get`
/// answers the resource itself, so the stub deserializes the document
/// directly. A wrapper that was applied anyway would yield a `Cluster` with
/// every field defaulted, which is exactly what the named cells below refuse.
#[tokio::test(flavor = "multi_thread")]
async fn the_fixture_cluster_detail_carries_every_field_the_pane_paints() {
    let detail = nuvem::services::sdk::gke::SdkGkeService::from_fixtures()
        .load_cluster_detail(PROJECT, "europe-west1", "platform-dev")
        .await
        .expect("the fixture client answers");

    assert_eq!(detail.name, "platform-dev", "detail: name");
    assert_eq!(detail.location, "europe-west1", "detail: location");
    assert_eq!(detail.status, GkeClusterStatus::Running, "detail: status");
    assert_eq!(
        detail.current_master_version.as_deref(),
        Some("1.31.5-gke.1023000"),
        "detail: master version"
    );
    assert_eq!(
        detail.current_node_version.as_deref(),
        Some("1.31.5-gke.1023000"),
        "detail: node version"
    );
    assert_eq!(
        detail.endpoint.as_deref(),
        Some("34.118.10.20"),
        "detail: endpoint"
    );
    assert!(!detail.private_cluster, "detail: private cluster");
    assert_eq!(
        detail.network.as_deref(),
        Some("shared-vpc"),
        "detail: network, the tail of the URL"
    );
    assert_eq!(
        detail.subnetwork.as_deref(),
        Some("gke-subnet"),
        "detail: subnetwork, the tail of the URL"
    );

    assert_eq!(
        detail
            .node_pools
            .iter()
            .map(|pool| pool.name.as_str())
            .collect::<Vec<_>>(),
        ["default-pool", "spot-pool"],
        "detail: node pools"
    );
    assert_eq!(
        detail.node_pools[0].machine_type.as_deref(),
        Some("e2-standard-4"),
        "default-pool: machine type, one level down in config"
    );
    assert_eq!(
        detail.node_pools[0].autoscaling.as_deref(),
        Some("1-6"),
        "default-pool: the autoscaling range, from three fields"
    );
    assert_eq!(
        detail.node_pools[0].node_count,
        Some(3),
        "default-pool: node count"
    );
    assert_eq!(
        detail.node_pools[1].autoscaling.as_deref(),
        Some("off"),
        "spot-pool has an autoscaling block with the switch off, which is not \
         the same as no block at all"
    );

    assert_eq!(
        detail
            .addons
            .iter()
            .map(|addon| (addon.name.as_str(), addon.enabled))
            .collect::<Vec<_>>(),
        [
            ("HTTP load balancing", true),
            ("Horizontal Pod Autoscaling", true),
            ("GCE Persistent Disk CSI Driver", true),
        ],
        "detail: addons, two of which are stated as `disabled: false`"
    );
    // Sorted before comparing, and the sort is not cosmetic: the generated
    // model holds `resource_labels` in a `HashMap` and the mapper collects it
    // in iteration order, so the pane's label order is different on every run
    // of the process. The CLI parser has the same `HashMap` behind it. That
    // instability is a defect in both mappers, reported by this slice rather
    // than fixed in it; what is asserted here is that the pair survives at all.
    let mut detail_labels = labels(&detail.labels);
    detail_labels.sort_unstable();
    assert_eq!(
        detail_labels,
        [("env", "dev"), ("team", "platform")],
        "detail: resource labels"
    );
    assert!(
        detail.raw_json.contains("platform-dev"),
        "the JSON tab is rendered from the client's own model, so it must at \
         least carry the cluster it describes; said: {}",
        detail.raw_json
    );
}

/// The GKE fixture rows and the CLI rows, from the same bytes.
///
/// `raw_json` is compared separately and loosely: the CLI pane pretty-prints
/// gcloud's document and the native pane serializes the generated model, so
/// the two texts differ by construction while every painted field must not.
#[tokio::test(flavor = "multi_thread")]
async fn the_fixture_gke_rows_match_the_cli_rows_parsed_from_the_same_bytes() {
    use nuvem::services::resources::gke::{parse_cluster_detail_json, parse_clusters_json};

    let service = nuvem::services::sdk::gke::SdkGkeService::from_fixtures();

    let native = service.load_clusters(PROJECT).await.expect("answers");
    let cli = parse_clusters_json(GKE_CLUSTERS_JSON).expect("the capture parses");

    // The two backends used to disagree on two cells, and both gaps were in
    // the *native mapper* rather than in the capture: `private-prod` reports
    // where it lives in the deprecated `zone` field, which `map_cluster` did
    // not read, and `legacy-unknown` reports no `currentNodeCount`, which
    // proto3 deserializes to 0 and `map_cluster` passed through. They were
    // pinned as divergences rather than papered over, and the flip to the
    // native backend in `--demo` is what made them cells an operator sees:
    // `every_service_fills_its_painted_columns_through_a_demo_session` caught
    // both. `cluster_location` and `cluster_node_count` close them, and the
    // comparison below is now whole-row rather than column-by-column, so a
    // third divergence cannot open without failing here first.
    assert_eq!(native.len(), cli.len(), "the same number of clusters");
    assert_eq!(
        native, cli,
        "every cluster row, field for field, from the same bytes"
    );
    assert_eq!(
        native[1].location, "europe-west1-b",
        "and the agreed value is the one the capture carries, not a blank \
         both sides happen to produce"
    );
    assert_eq!(
        native[2].current_node_count, None,
        "likewise: an unreported node count is a dash on both backends"
    );

    let mut native = service
        .load_cluster_detail(PROJECT, "europe-west1", "platform-dev")
        .await
        .expect("answers");
    let mut cli = parse_cluster_detail_json(GKE_CLUSTER_DETAIL_JSON).expect("the capture parses");
    assert!(
        !native.raw_json.is_empty(),
        "the native pane has a JSON tab"
    );
    assert!(!cli.raw_json.is_empty(), "so does the CLI pane");
    native.raw_json.clear();
    cli.raw_json.clear();
    // Both sides read `resourceLabels` out of a `HashMap`, so both orders are
    // whatever that run's hasher produced; sorted here so the comparison
    // measures content rather than the hasher. The instability itself is
    // reported with this slice.
    native.labels.sort_by(|a, b| a.key.cmp(&b.key));
    cli.labels.sort_by(|a, b| a.key.cmp(&b.key));
    assert_eq!(native, cli, "and the same cluster detail, field for field");
}

// ---------------------------------------------------------------------------
// Secret Manager
// ---------------------------------------------------------------------------

const SECRETS_JSON: &str = include_str!("fixtures/secrets.json");
const SECRET_VERSIONS_JSON: &str = include_str!("fixtures/secret_versions.json");
const SECRET_DETAIL_JSON: &str = include_str!("fixtures/secret_detail.json");

/// Every cell of every secret row.
///
/// `replication` is a rendered label rather than a field: it comes from a
/// `oneof` whose two arms are `automatic` and `userManaged`, and the
/// user-managed arm's replica locations are two levels below that. A capture
/// that lost the oneof still yields a named, timestamped secret, and only this
/// cell says otherwise.
#[tokio::test(flavor = "multi_thread")]
async fn the_fixture_secrets_carry_every_column_the_table_paints() {
    let rows = nuvem::services::sdk::secrets::SdkSecretsService::from_fixtures()
        .load_secrets(PROJECT)
        .await
        .expect("the fixture client answers");

    assert_eq!(
        rows.iter().map(|row| row.name.as_str()).collect::<Vec<_>>(),
        ["api-signing-key", "db-password", "unlabelled-token"],
        "the capture's three secrets, shortened to their last path segment"
    );

    assert_eq!(
        rows[0].create_time.as_deref(),
        Some("2026-02-11T09:14:22Z"),
        "api-signing-key: create time"
    );
    assert_eq!(
        labels(&rows[0].labels),
        [("env", "prod"), ("owner", "platform")],
        "api-signing-key: labels, sorted by key"
    );
    assert_eq!(
        rows[0].replication, "Automatic",
        "api-signing-key: replication"
    );

    assert_eq!(
        rows[1].create_time.as_deref(),
        Some("2026-03-02T17:41:08Z"),
        "db-password: create time"
    );
    assert_eq!(
        labels(&rows[1].labels),
        [("env", "staging")],
        "db-password: labels"
    );
    assert_eq!(
        rows[1].replication, "User-managed (europe-west1, europe-west4)",
        "db-password names its replica locations, which live under the \
         oneof's user-managed arm"
    );

    assert_eq!(
        rows[2].create_time.as_deref(),
        Some("2026-05-20T06:00:00Z"),
        "unlabelled-token: create time"
    );
    assert_eq!(
        labels(&rows[2].labels),
        [] as [(&str, &str); 0],
        "unlabelled-token: labels"
    );
    assert_eq!(
        rows[2].replication, "Automatic",
        "unlabelled-token: replication"
    );
}

/// Every cell of every version row.
#[tokio::test(flavor = "multi_thread")]
async fn the_fixture_secret_versions_carry_every_column_the_pane_paints() {
    let rows = nuvem::services::sdk::secrets::SdkSecretsService::from_fixtures()
        .load_secret_versions(PROJECT, "api-signing-key")
        .await
        .expect("the fixture client answers");

    assert_eq!(
        rows.iter().map(|row| row.name.as_str()).collect::<Vec<_>>(),
        ["3", "2", "1", "0"],
        "the capture's four versions, newest first"
    );
    assert_eq!(
        rows.iter().map(|row| row.state.clone()).collect::<Vec<_>>(),
        [
            SecretVersionState::Enabled,
            SecretVersionState::Disabled,
            SecretVersionState::Destroyed,
            SecretVersionState::Unknown("STATE_UNSPECIFIED".to_string()),
        ],
        "one version per state the pane can show, including the sentinel a \
         proto3 zero-valued enum arrives as"
    );
    assert_eq!(
        rows.iter()
            .map(|row| row.create_time.as_deref())
            .collect::<Vec<_>>(),
        [
            Some("2026-06-01T11:02:45Z"),
            Some("2026-04-18T08:30:00Z"),
            Some("2026-02-11T09:14:22Z"),
            Some("2026-02-11T09:00:00Z"),
        ],
        "every version: create time"
    );
}

/// Every field the secret detail pane paints, from the single-secret capture.
#[tokio::test(flavor = "multi_thread")]
async fn the_fixture_secret_detail_carries_every_field_the_pane_paints() {
    let detail = nuvem::services::sdk::secrets::SdkSecretsService::from_fixtures()
        .load_secret_detail(PROJECT, "api-signing-key")
        .await
        .expect("the fixture client answers");

    assert_eq!(detail.name, "api-signing-key", "detail: name");
    assert_eq!(
        detail.create_time.as_deref(),
        Some("2026-02-11T09:14:22Z"),
        "detail: create time"
    );
    assert_eq!(
        labels(&detail.labels),
        [("env", "prod"), ("owner", "platform")],
        "detail: labels"
    );
    assert_eq!(
        labels(&detail.annotations),
        [("managed-by", "terraform")],
        "detail: annotations, which the row does not carry at all"
    );
    assert_eq!(
        detail.replication, "User-managed (europe-west1, europe-west4)",
        "detail: replication"
    );
    assert_eq!(
        detail.rotation.as_deref(),
        Some("every 7776000s, next 2026-08-11T09:14:22Z"),
        "detail: rotation, rendered from a duration and a timestamp"
    );
    assert_eq!(
        detail.expire_time.as_deref(),
        Some("2027-02-11T09:14:22Z"),
        "detail: expire time, the absolute arm of the expiration oneof"
    );
    assert!(
        detail.versions.is_empty(),
        "the pane fetches versions separately, so this stays empty on both \
         backends"
    );
    assert!(
        detail.raw_json.contains("api-signing-key"),
        "the JSON tab must carry the secret it describes; said: {}",
        detail.raw_json
    );
}

/// The Secret Manager fixture rows and the CLI rows, from the same bytes.
#[tokio::test(flavor = "multi_thread")]
async fn the_fixture_secret_rows_match_the_cli_rows_parsed_from_the_same_bytes() {
    use nuvem::services::resources::secrets::{
        parse_secret_detail_json, parse_secret_versions_json, parse_secrets_json,
    };

    let service = nuvem::services::sdk::secrets::SdkSecretsService::from_fixtures();

    assert_eq!(
        service.load_secrets(PROJECT).await.expect("answers"),
        parse_secrets_json(SECRETS_JSON).expect("the capture parses"),
        "the two backends must paint the same secrets table from the same file"
    );
    assert_eq!(
        service
            .load_secret_versions(PROJECT, "api-signing-key")
            .await
            .expect("answers"),
        parse_secret_versions_json(SECRET_VERSIONS_JSON).expect("the capture parses"),
        "and the same versions list"
    );

    let mut native = service
        .load_secret_detail(PROJECT, "api-signing-key")
        .await
        .expect("answers");
    let mut cli = parse_secret_detail_json(SECRET_DETAIL_JSON).expect("the capture parses");
    assert!(
        !native.raw_json.is_empty(),
        "the native pane has a JSON tab"
    );
    assert!(!cli.raw_json.is_empty(), "so does the CLI pane");
    native.raw_json.clear();
    cli.raw_json.clear();
    assert_eq!(native, cli, "and the same secret detail, field for field");
}

// ---------------------------------------------------------------------------
// IAM
// ---------------------------------------------------------------------------

const SERVICE_ACCOUNTS_JSON: &str = include_str!("fixtures/iam_service_accounts.json");
const IAM_POLICY_JSON: &str = include_str!("fixtures/iam_policy.json");

/// Every cell of every service-account row.
///
/// `display_name` carries the one interpretation on this table: the generated
/// model answers `""` where the CLI's JSON omitted the key, and `Some("")`
/// would paint a blank cell while every `is_some()` downstream believed there
/// was a name. The capture holds both an absent and an empty one.
#[tokio::test(flavor = "multi_thread")]
async fn the_fixture_service_accounts_carry_every_column_the_table_paints() {
    let rows = nuvem::services::sdk::iam::SdkIamService::from_fixtures()
        .load_service_accounts(PROJECT)
        .await
        .expect("the fixture client answers");

    assert_eq!(
        rows.iter()
            .map(|row| row.email.as_str())
            .collect::<Vec<_>>(),
        [
            "sa-vault@dev-project.iam.gserviceaccount.com",
            "old-deployer@dev-project.iam.gserviceaccount.com",
            "no-display-name@dev-project.iam.gserviceaccount.com",
            "empty-display-name@dev-project.iam.gserviceaccount.com",
            "",
        ],
        "the capture's five accounts, including the one with no email at all"
    );

    assert_eq!(
        rows[0].display_name.as_deref(),
        Some("SA used by the vault node pool"),
        "sa-vault: display name"
    );
    assert!(!rows[0].disabled, "sa-vault: disabled");
    assert_eq!(
        rows[0].unique_id, "110727432291788759739",
        "sa-vault: unique id"
    );

    assert_eq!(
        rows[1].display_name.as_deref(),
        Some("Retired deploy account"),
        "old-deployer: display name"
    );
    assert!(
        rows[1].disabled,
        "old-deployer is disabled, and the column is the whole point of the row"
    );
    assert_eq!(
        rows[1].unique_id, "104405669981102690742",
        "old-deployer: unique id"
    );

    assert_eq!(
        rows[2].display_name, None,
        "an omitted display name is no display name"
    );
    assert_eq!(
        rows[3].display_name, None,
        "and so is an empty one: `Some(\"\")` paints a blank cell that every \
         `is_some()` downstream reads as a name"
    );
    assert_eq!(
        rows[4].unique_id, "",
        "the account with nothing on it still lists, rather than dropping the \
         row"
    );
}

/// Every cell of every policy row.
///
/// The policy is one document expanded into one row per member, so the row
/// count is already derived; what each row says about a member is the part a
/// count cannot see. The capture carries one member of every kind the table
/// distinguishes, plus a conditional binding and a deleted principal.
#[tokio::test(flavor = "multi_thread")]
async fn the_fixture_policy_rows_carry_every_column_the_table_paints() {
    let rows = nuvem::services::sdk::iam::SdkIamService::from_fixtures()
        .load_policy(PROJECT)
        .await
        .expect("the fixture client answers");

    assert_eq!(
        rows.iter()
            .map(|row| (row.role.as_str(), row.principal.as_str()))
            .collect::<Vec<_>>(),
        [
            (
                "roles/artifactregistry.admin",
                "sa-gke@dev-project.iam.gserviceaccount.com"
            ),
            ("roles/artifactregistry.admin", "alice@example.com"),
            ("roles/artifactregistry.admin", "platform@example.com"),
            ("roles/storage.objectViewer", "allUsers"),
            ("roles/viewer", "allAuthenticatedUsers"),
            ("roles/viewer", "example.com"),
            (
                "roles/editor",
                "gone@dev-project.iam.gserviceaccount.com?uid=104405669981102690742"
            ),
            ("roles/editor", "bob@example.com?uid=9"),
            ("roles/compute.admin", "carol@example.com"),
            (
                "projects/dev-project/roles/artifactregistry.admin",
                "//iam.googleapis.com/projects/1/locations/global/workloadIdentityPools/gh/*"
            ),
            (
                "projects/dev-project/roles/artifactregistry.admin",
                "//iam.googleapis.com/locations/global/workforcePools/wf/subject/user:erin@example.com"
            ),
            ("", "dave@example.com"),
        ],
        "one row per member, carrying the binding's role -- including the \
         binding that names no role, whose members must still be listed"
    );

    assert_eq!(
        rows.iter().map(|row| row.kind.clone()).collect::<Vec<_>>(),
        [
            IamMemberKind::ServiceAccount,
            IamMemberKind::User,
            IamMemberKind::Group,
            IamMemberKind::AllUsers,
            IamMemberKind::AllAuthenticatedUsers,
            IamMemberKind::Domain,
            IamMemberKind::ServiceAccount,
            IamMemberKind::User,
            IamMemberKind::User,
            IamMemberKind::PrincipalSet,
            IamMemberKind::Principal,
            IamMemberKind::User,
        ],
        "every member kind the table distinguishes"
    );

    assert_eq!(
        rows.iter()
            .filter(|row| row.deleted)
            .map(|row| row.raw_member.as_str())
            .collect::<Vec<_>>(),
        [
            "deleted:serviceAccount:gone@dev-project.iam.gserviceaccount.com?uid=104405669981102690742",
            "deleted:user:bob@example.com?uid=9",
        ],
        "the two `deleted:` principals, and no others"
    );

    assert_eq!(
        rows[8].condition.as_deref(),
        Some("expires-2027"),
        "a conditional binding's title travels onto the row: a grant shown \
         without its condition claims broader access than the one that exists"
    );
    assert_eq!(
        rows.iter().filter(|row| row.condition.is_some()).count(),
        1,
        "and only that binding carries one"
    );
}

/// The IAM fixture rows and the CLI rows, from the same bytes.
#[tokio::test(flavor = "multi_thread")]
async fn the_fixture_iam_rows_match_the_cli_rows_parsed_from_the_same_bytes() {
    use nuvem::services::resources::iam::{parse_policy_json, parse_service_accounts_json};

    let service = nuvem::services::sdk::iam::SdkIamService::from_fixtures();

    assert_eq!(
        service
            .load_service_accounts(PROJECT)
            .await
            .expect("answers"),
        parse_service_accounts_json(SERVICE_ACCOUNTS_JSON).expect("the capture parses"),
        "the two backends must paint the same service accounts table from the \
         same file"
    );
    assert_eq!(
        service.load_policy(PROJECT).await.expect("answers"),
        parse_policy_json(IAM_POLICY_JSON).expect("the capture parses"),
        "and the same policy table"
    );
}

// ---------------------------------------------------------------------------
// Cloud Functions
// ---------------------------------------------------------------------------

const CLOUD_FUNCTIONS_JSON: &str = include_str!("fixtures/cloud_functions.json");

/// Every cell of every function row.
///
/// Two columns here are not fields. `region` is read out of the resource path,
/// because the API carries no region and a function still deploying has no URL
/// to read one from; `state` and `environment` are enums whose unrecognised
/// arms the capture exercises deliberately.
#[tokio::test(flavor = "multi_thread")]
async fn the_fixture_functions_carry_every_column_the_table_paints() {
    let rows = nuvem::services::sdk::functions::SdkFunctionsService::from_fixtures()
        .load_functions(PROJECT)
        .await
        .expect("the fixture client answers");

    assert_eq!(
        rows.iter().map(|row| row.name.as_str()).collect::<Vec<_>>(),
        [
            "api-gateway",
            "legacy-thumbnailer",
            "image-resizer",
            "detached-worker",
            "drifting-webhook",
            "quarantined-job",
            "bare-function",
        ],
        "the capture's seven functions, shortened to their last path segment"
    );
    assert_eq!(
        rows.iter()
            .map(|row| row.region.as_str())
            .collect::<Vec<_>>(),
        [
            "europe-west1",
            "us-central1",
            "europe-west4",
            "asia-northeast1",
            "us-east1",
            "us-east1",
            "us-west2",
        ],
        "every region, read out of the resource path rather than from a field"
    );
    assert_eq!(
        rows.iter().map(|row| row.state.clone()).collect::<Vec<_>>(),
        [
            CloudFunctionState::Active,
            CloudFunctionState::Active,
            CloudFunctionState::Deploying,
            CloudFunctionState::DetachFailed,
            CloudFunctionState::Unknown,
            CloudFunctionState::Unrecognized("SUSPENDED".to_string()),
            CloudFunctionState::Unspecified,
        ],
        "every state, including a value outside the enum and the sentinel a \
         proto3 zero-valued enum arrives as"
    );
    assert_eq!(
        rows.iter()
            .map(|row| row.environment.clone())
            .collect::<Vec<_>>(),
        [
            CloudFunctionEnvironment::Gen2,
            CloudFunctionEnvironment::Gen1,
            CloudFunctionEnvironment::Gen2,
            CloudFunctionEnvironment::Unspecified,
            CloudFunctionEnvironment::Gen2,
            CloudFunctionEnvironment::Unrecognized("GEN_3".to_string()),
            CloudFunctionEnvironment::Unspecified,
        ],
        "every environment"
    );
    assert_eq!(
        rows.iter()
            .map(|row| row.url.as_deref())
            .collect::<Vec<_>>(),
        [
            Some("https://europe-west1-dev-project.cloudfunctions.net/api-gateway"),
            Some("https://us-central1-dev-project.cloudfunctions.net/legacy-thumbnailer"),
            None,
            None,
            None,
            None,
            None,
        ],
        "a function with no URL has none, rather than an empty string that \
         paints a blank link"
    );
    assert_eq!(
        rows.iter()
            .map(|row| row.update_time.as_deref())
            .collect::<Vec<_>>(),
        [
            Some("2026-07-14T08:31:05Z"),
            Some("2025-11-02T16:04:44Z"),
            Some("2026-08-01T11:59:12Z"),
            Some("2026-06-21T04:12:00Z"),
            Some("2026-05-09T22:47:31Z"),
            Some("2026-07-30T13:20:00Z"),
            None,
        ],
        "every update time"
    );
    assert_eq!(
        labels(&rows[0].labels),
        [("env", "prod"), ("team", "platform")],
        "api-gateway: labels, sorted by key"
    );
    assert_eq!(
        labels(&rows[6].labels),
        [] as [(&str, &str); 0],
        "bare-function: labels"
    );
}

/// The Cloud Functions fixture rows and the CLI rows, from the same bytes.
#[tokio::test(flavor = "multi_thread")]
async fn the_fixture_function_rows_match_the_cli_rows_parsed_from_the_same_bytes() {
    use nuvem::services::resources::functions::parse_functions_json;

    assert_eq!(
        nuvem::services::sdk::functions::SdkFunctionsService::from_fixtures()
            .load_functions(PROJECT)
            .await
            .expect("answers"),
        parse_functions_json(CLOUD_FUNCTIONS_JSON).expect("the capture parses"),
        "the two backends must paint the same functions table from the same file"
    );
}

// ---------------------------------------------------------------------------
// Cloud Logging
// ---------------------------------------------------------------------------

const LOG_ENTRIES_JSON: &str = include_str!("fixtures/log_entries.json");

/// Every cell of every log entry, through both reads.
///
/// The payload is the column a count cannot see: one entry carries a
/// `textPayload` and the other a `jsonPayload`, the derived `message` differs
/// per case, and an entry whose payload was lost still has an id, a timestamp
/// and a severity.
#[tokio::test(flavor = "multi_thread")]
async fn the_fixture_log_entries_carry_every_column_the_viewer_paints() {
    use nuvem_domain::gcp::logging::{LogPayload, LogQuery, LogSeverity, LogTimeRange};

    let outcome = nuvem::services::sdk::logging::SdkLoggingService::from_fixtures()
        .read_recent(
            PROJECT,
            &LogQuery::ProjectLogs,
            LogSeverity::Default,
            LogTimeRange::DEFAULT,
            100,
        )
        .await
        .expect("the fixture client answers");

    assert_eq!(
        outcome.dropped, 0,
        "both captured entries carry an insertId, so neither is dropped"
    );
    assert_eq!(
        outcome
            .entries
            .iter()
            .map(|entry| entry.insert_id.as_str())
            .collect::<Vec<_>>(),
        ["aaa-1", "bbb-2"],
        "the capture is newest-first and `read_recent` reverses it, so the \
         buffer's oldest-first convention holds"
    );

    let first = &outcome.entries[0];
    assert_eq!(
        first.timestamp, "2026-07-19T10:00:00.000000000Z",
        "aaa-1: timestamp, in the nanosecond form a resume filter can parse"
    );
    assert_eq!(first.severity, LogSeverity::Error, "aaa-1: severity");
    assert_eq!(first.resource_type, "gce_instance", "aaa-1: resource type");
    assert_eq!(
        first
            .resource_labels
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
            .collect::<Vec<_>>(),
        [("instance_id", "1001"), ("zone", "europe-west1-b")],
        "aaa-1: resource labels"
    );
    assert_eq!(
        first.log_name, "projects/dev-project/logs/compute.googleapis.com%2Factivity",
        "aaa-1: log name, still percent-encoded as the API returns it"
    );
    assert_eq!(
        first.payload,
        LogPayload::Text("boot failed[2J for api-dev-1".to_string()),
        "aaa-1: the text payload, control sequence and all"
    );
    assert_eq!(
        first.message, "boot failed[2J for api-dev-1",
        "aaa-1: the one-line message derived from that payload"
    );
    assert_eq!(
        first
            .labels
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
            .collect::<Vec<_>>(),
        [("run", "1")],
        "aaa-1: entry labels"
    );

    let second = &outcome.entries[1];
    assert_eq!(
        second.timestamp, "2026-07-19T10:00:01.000000000Z",
        "bbb-2: timestamp"
    );
    assert_eq!(second.severity, LogSeverity::Info, "bbb-2: severity");
    assert_eq!(
        second.payload,
        LogPayload::Json("{\n  \"message\": \"started\",\n  \"pid\": 42\n}".to_string()),
        "bbb-2: the structured payload, pretty-printed"
    );
    assert_eq!(
        second.message, "started",
        "bbb-2: the message a structured payload's own `message` field supplies"
    );
    assert!(second.labels.is_empty(), "bbb-2: entry labels");
}

/// The log entries the fixture client reads and the ones the CLI parser reads,
/// from the same bytes.
///
/// Compared through `read_since` rather than `read_recent` because that is the
/// read that does not reverse: the CLI service reverses in its own
/// `read_recent`, and comparing the two reversing paths would hide a backend
/// that had stopped reversing.
#[tokio::test(flavor = "multi_thread")]
async fn the_fixture_log_entries_match_the_cli_entries_parsed_from_the_same_bytes() {
    use nuvem::services::resources::logging::parse_log_entries_json;
    use nuvem_domain::gcp::logging::{LogCursor, LogQuery, LogSeverity};

    let native = nuvem::services::sdk::logging::SdkLoggingService::from_fixtures()
        .read_since(
            PROJECT,
            &LogQuery::ProjectLogs,
            LogSeverity::Default,
            &LogCursor {
                timestamp: "2026-01-01T00:00:00Z".to_string(),
                insert_ids: Vec::new(),
            },
            100,
        )
        .await
        .expect("answers");

    assert_eq!(
        native,
        parse_log_entries_json(LOG_ENTRIES_JSON).expect("the capture parses"),
        "the two backends must paint the same log viewer from the same file"
    );
}

// ---------------------------------------------------------------------------
// FinOps
// ---------------------------------------------------------------------------

/// Every cell of the recommendation each captured pair answers.
///
/// The monthly saving is the cell that carries two corrections: the API states
/// a saving as a NEGATIVE cost over the projection's OWN duration, so a figure
/// that reads positive and monthly is evidence that both happened.
#[tokio::test(flavor = "multi_thread")]
async fn the_fixture_recommendations_carry_every_cell_the_cards_paint() {
    use nuvem_domain::gcp::finops::{RecommendationState, RecommenderId};

    let service = nuvem::services::sdk::finops::SdkFinOpsService::from_fixtures();

    for (recommender, location, description, priority, xor_group, saving, resource) in [
        (
            RecommenderId::IdleVm,
            "europe-west1-b",
            "Save cost by stopping idle VM instance 'worker-dev-1'.",
            "P2",
            None,
            "USD 24.50",
            "//compute.googleapis.com/projects/dev-project/zones/europe-west1-b/instances/worker-dev-1",
        ),
        (
            RecommenderId::MachineType,
            "europe-west1-b",
            "Save cost by changing machine type from e2-medium to e2-small.",
            "P3",
            Some("e5f4a3b2-1c0d-4e9f-8a7b-6c5d4e3f2a1b"),
            "USD 8.25",
            "//compute.googleapis.com/projects/dev-project/zones/europe-west1-b/instances/api-dev-1",
        ),
        (
            RecommenderId::IdleDisk,
            "europe-west1-b",
            "Save cost by snapshotting and then deleting idle persistent disk 'detached-legacy'.",
            "P2",
            None,
            "USD 3.40",
            "//compute.googleapis.com/projects/dev-project/zones/europe-west1-b/disks/detached-legacy",
        ),
        (
            RecommenderId::UnusedAddress,
            "europe-west1",
            "Save cost by deleting idle external IP address 'legacy-lb-ip'.",
            "P4",
            None,
            "USD 7.30",
            "//compute.googleapis.com/projects/dev-project/regions/europe-west1/addresses/legacy-lb-ip",
        ),
    ] {
        let rows = service
            .load_recommendations(PROJECT, recommender, location)
            .await
            .unwrap_or_else(|error| panic!("{recommender:?} at {location}: {error}"));

        assert_eq!(
            rows.len(),
            1,
            "{recommender:?} at {location} answers exactly one recommendation"
        );
        let row = &rows[0];
        assert!(
            row.name.contains(recommender.id()) && row.name.contains(location),
            "{recommender:?}: the recommendation's own name must name the pair \
             it was served for; said: {}",
            row.name
        );
        assert_eq!(row.description, description, "{recommender:?}: description");
        assert_eq!(
            row.state,
            RecommendationState::Active,
            "{recommender:?}: state"
        );
        assert_eq!(
            row.priority.as_deref(),
            Some(priority),
            "{recommender:?}: priority"
        );
        assert_eq!(
            row.xor_group.as_deref(),
            xor_group,
            "{recommender:?}: xor group"
        );
        assert_eq!(
            row.monthly.as_ref().map(|money| money.render()),
            Some(saving.to_string()),
            "{recommender:?}: the monthly saving, sign-corrected and scaled"
        );
        assert_eq!(
            row.last_refresh.as_deref(),
            Some("2026-08-24T03:12:07Z"),
            "{recommender:?}: last refresh"
        );
        assert_eq!(
            row.target_resources,
            [resource.to_string()],
            "{recommender:?}: target resources"
        );
    }
}

/// Two different locations must not answer the same rows.
///
/// This is the double-count trap, and it is the reason the stub keys on the
/// `(recommender, location)` pair rather than on the recommender alone.
/// `FinOpsState::recommendations` flattens a recommender's cells across every
/// location it asked about, so a stub that served `finops_idle_vms.json` for
/// every zone would list one recommendation once per zone and the savings
/// total would count it that many times. The captured zones come from the demo
/// compute and disk fixtures, which name `europe-west1-b` and `europe-west1-c`;
/// only the first has anything to save.
#[tokio::test(flavor = "multi_thread")]
async fn a_second_location_gets_different_rows_rather_than_the_same_ones_again() {
    use nuvem_domain::gcp::finops::RecommenderId;

    let service = nuvem::services::sdk::finops::SdkFinOpsService::from_fixtures();

    let captured = service
        .load_recommendations(PROJECT, RecommenderId::IdleVm, "europe-west1-b")
        .await
        .expect("answers");
    let elsewhere = service
        .load_recommendations(PROJECT, RecommenderId::IdleVm, "europe-west1-c")
        .await
        .expect("answers");

    assert_eq!(
        captured.len(),
        1,
        "europe-west1-b has one idle VM to report"
    );
    assert_ne!(
        captured, elsewhere,
        "the two zones must not answer the same rows: the FinOps screen adds \
         a recommender's rows up across every location it asked about, so \
         repeating them would report savings that do not exist"
    );
    assert!(
        elsewhere.is_empty(),
        "and the second zone's answer is an empty list -- 'asked, nothing to \
         save' -- rather than a repeat; got {elsewhere:?}"
    );

    // The location is half the key, not a suffix on the recommender: the
    // regional recommender's own region answers nothing for a zonal one, and
    // a zone answers nothing for the regional one.
    assert!(
        service
            .load_recommendations(PROJECT, RecommenderId::IdleVm, "europe-west1")
            .await
            .expect("answers")
            .is_empty(),
        "a zonal recommender asked about a region has nothing captured"
    );
    assert!(
        service
            .load_recommendations(PROJECT, RecommenderId::UnusedAddress, "europe-west1-b")
            .await
            .expect("answers")
            .is_empty(),
        "and a regional recommender asked about a zone likewise"
    );
}

/// The recommendations the fixture client reads and the ones the CLI path
/// reads, pair for pair.
///
/// Compared through `DemoCommandRunner` rather than through a parser call,
/// because the CLI path's answer is chosen by the same `(recommender,
/// location)` pair: routing is half of what has to agree here, and a
/// comparison that handed both sides the same file would not test it.
#[tokio::test(flavor = "multi_thread")]
async fn the_fixture_recommendations_match_the_cli_rows_for_every_pair() {
    use nuvem::services::resources::finops::GcloudFinOpsService;
    use nuvem_domain::gcp::configuration::{GcloudConfigurationName, GcloudContext};
    use nuvem_domain::gcp::finops::RecommenderId;

    let context = GcloudContext {
        configuration: GcloudConfigurationName::new("dev"),
        account: Some("dev@example.com".to_string()),
        project: Some(PROJECT.to_string()),
        region: None,
        zone: None,
    };
    let cli = GcloudFinOpsService::new(nuvem::services::demo::DemoCommandRunner::new());
    let native = nuvem::services::sdk::finops::SdkFinOpsService::from_fixtures();

    for recommender in RecommenderId::ALL {
        for location in ["europe-west1-b", "europe-west1-c", "europe-west1"] {
            assert_eq!(
                native
                    .load_recommendations(PROJECT, recommender, location)
                    .await
                    .expect("the fixture client answers"),
                cli.load_recommendations(&context, recommender, location)
                    .expect("the demo runner answers"),
                "the two backends must agree for {recommender:?} at {location}, \
                 including on which pairs have nothing to report"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Region and zone discovery
// ---------------------------------------------------------------------------

const REGIONS_JSON: &str = include_str!("fixtures/gcloud_compute_regions.json");
const ZONES_JSON: &str = include_str!("fixtures/gcloud_compute_zones.json");

/// Every cell of every region and zone row.
///
/// `status` is taken by name and not by number on both paths -- `value()`
/// answers `1`, not `UP`, and the picker's incremental filter matches on what
/// is painted -- and a zone's region arrives as a URL that has to be shortened
/// to match the region rows it is filtered against.
#[tokio::test(flavor = "multi_thread")]
async fn the_fixture_regions_and_zones_carry_every_column_the_pickers_paint() {
    let service = nuvem::services::sdk::discovery::SdkDiscoveryService::from_fixtures();

    let regions = service
        .load_regions(PROJECT)
        .await
        .expect("the fixture client answers");
    assert_eq!(
        regions
            .iter()
            .map(|row| (row.name.as_str(), row.status.as_deref()))
            .collect::<Vec<_>>(),
        [("europe-west1", Some("UP")), ("us-central1", Some("UP")),],
        "both regions, each with the status NAME rather than its wire number"
    );

    let zones = service
        .load_zones(PROJECT)
        .await
        .expect("the fixture client answers");
    assert_eq!(
        zones
            .iter()
            .map(|row| (
                row.name.as_str(),
                row.region.as_deref(),
                row.status.as_deref()
            ))
            .collect::<Vec<_>>(),
        [
            ("europe-west1-b", Some("europe-west1"), Some("UP")),
            ("us-central1-a", Some("us-central1"), Some("UP")),
        ],
        "both zones, each with its region shortened out of the URL the API \
         answers"
    );
}

/// The discovery fixture rows and the CLI rows, from the same bytes.
#[tokio::test(flavor = "multi_thread")]
async fn the_fixture_discovery_rows_match_the_cli_rows_parsed_from_the_same_bytes() {
    use nuvem::services::gcloud::discovery::{parse_regions_json, parse_zones_json};

    let service = nuvem::services::sdk::discovery::SdkDiscoveryService::from_fixtures();

    assert_eq!(
        service.load_regions(PROJECT).await.expect("answers"),
        parse_regions_json(REGIONS_JSON).expect("the capture parses"),
        "the two backends must paint the same region picker from the same file"
    );
    assert_eq!(
        service.load_zones(PROJECT).await.expect("answers"),
        parse_zones_json(ZONES_JSON).expect("the capture parses"),
        "and the same zone picker"
    );
}

/// A row carrying both a zone and a region is filed zonally.
///
/// Asserted directly rather than through a capture, because no capture can
/// assert it: every real Compute resource is zonal or regional and never both,
/// so reversing the precedence leaves every fixture-driven test in this file
/// green. That was measured -- it is the one mutation of `scope` the whole
/// suite missed.
///
/// The precedence is not arbitrary. `map_disk` paints LOCATION with
/// `disk.zone.as_deref().or(disk.region.as_deref())`, so a row that somehow
/// carried both would paint its zone; filing it regionally would put it under a
/// scope its own cell contradicts.
#[test]
fn a_row_naming_both_a_zone_and_a_region_is_filed_under_the_zone() {
    use nuvem::services::demo::sdk::scope;

    assert_eq!(
        scope(
            Some("europe-west1-b".to_string()),
            Some("europe-west1".to_string())
        ),
        "zones/europe-west1-b",
        "zone wins, matching the precedence map_disk reads the two fields in"
    );
    assert_eq!(
        scope(None, Some("europe-west1".to_string())),
        "regions/europe-west1",
        "a row with only a region is regional"
    );
    assert_eq!(
        scope(Some("europe-west1-b".to_string()), None),
        "zones/europe-west1-b",
        "a row with only a zone is zonal"
    );
    assert_eq!(
        scope(None, None),
        "global",
        "a row that is neither keeps the API's own name for that scope, rather \
         than being dropped and painting one row fewer than the CLI backend"
    );
}

// ---------------------------------------------------------------------------
// Cloud Run
// ---------------------------------------------------------------------------

const CLOUD_RUN_SERVICES_JSON: &str = include_str!("fixtures/cloud_run_services.json");
const CLOUD_RUN_SERVICE_DETAIL_JSON: &str = include_str!("fixtures/cloud_run_service_detail.json");
const CLOUD_RUN_REVISIONS_JSON: &str = include_str!("fixtures/cloud_run_revisions.json");
const CLOUD_RUN_REVISION_DETAIL_JSON: &str =
    include_str!("fixtures/cloud_run_revision_detail.json");

/// Every cell of every service row, including the traffic cell the re-shaping
/// is easiest to get wrong.
#[tokio::test(flavor = "multi_thread")]
async fn the_fixture_cloud_run_services_carry_every_column_the_table_paints() {
    let rows = nuvem::services::sdk::cloud_run::SdkCloudRunService::from_fixtures()
        .load_services(PROJECT)
        .await
        .expect("the fixture client answers");

    assert_eq!(
        rows.iter()
            .map(|row| (row.name.as_str(), row.region.as_str()))
            .collect::<Vec<_>>(),
        [
            ("orders-api", "europe-west1"),
            ("billing-worker", "us-central1"),
            ("checkout-api", "europe-west1"),
            ("payments-api", "europe-west1"),
            ("legacy-frontend", "asia-northeast1"),
        ],
        "the capture's five services, each with its name and location read out \
         of the v2 resource path -- v2 carries neither as its own field"
    );

    assert_eq!(
        rows[0].url.as_deref(),
        Some("https://orders-api-abcde-ew.a.run.app"),
        "orders-api: URL"
    );
    assert_eq!(
        rows[0].latest_ready_revision.as_deref(),
        Some("orders-api-00007-tabc"),
        "orders-api: the ready revision, shortened out of the resource path"
    );
    assert_eq!(
        rows[0].latest_created_revision.as_deref(),
        Some("orders-api-00007-tabc"),
        "orders-api: the created revision"
    );
    assert_eq!(
        rows[0].traffic_summary, "orders-api-00007-tabc 100%",
        "orders-api: the traffic cell is a BARE revision name and a percentage. \
         `map_service` hands `trafficStatuses[].revision` to \
         `traffic_summary_from` untouched, where `map_service_detail` shortens \
         the same field first -- so a full `projects/.../revisions/x` path here \
         would paint the whole resource path into this cell while the detail \
         pane still read correctly"
    );
    assert_eq!(
        rows[0].status,
        CloudRunServiceStatus::Ready,
        "orders-api: ready"
    );
    assert_eq!(
        rows[0].ready_message, None,
        "orders-api: a succeeded terminal condition carries no message"
    );

    assert_eq!(
        rows[3].traffic_summary, "payments-api-00019-old0 80%",
        "payments-api: the busiest of two targets, not the latest one"
    );
    assert_eq!(
        rows[3].status,
        CloudRunServiceStatus::Reconciling,
        "payments-api: CONDITION_RECONCILING is still settling"
    );
    assert_eq!(
        rows[3].ready_message.as_deref(),
        Some("Deploying revision payments-api-00020-new0."),
        "payments-api: the terminal condition's message"
    );

    assert_eq!(
        rows[1].status,
        CloudRunServiceStatus::NotReady,
        "billing-worker: CONDITION_FAILED stating no reason is the \
         not-yet-ready one, painted \"Not Ready\" toned Warning"
    );
    assert_eq!(
        rows[2].status,
        CloudRunServiceStatus::Failed,
        "checkout-api: the same CONDITION_FAILED, but stating a \
         `revisionReason` -- a cause makes it a failure, painted \"Failed\" \
         toned Degraded. These two rows differ in the reason and nothing else"
    );

    assert_eq!(
        rows[4].url, None,
        "legacy-frontend: a service with no URL shows none rather than an \
         empty string"
    );
    assert_eq!(
        rows[4].traffic_summary, "-",
        "legacy-frontend: no traffic status at all"
    );
}

/// Every field of the service detail pane.
#[tokio::test(flavor = "multi_thread")]
async fn the_fixture_cloud_run_service_detail_carries_every_field_the_pane_paints() {
    let detail = nuvem::services::sdk::cloud_run::SdkCloudRunService::from_fixtures()
        .load_service_detail(PROJECT, "europe-west1", "orders-api")
        .await
        .expect("the fixture client answers");

    assert_eq!(detail.name, "orders-api");
    assert_eq!(detail.region, "europe-west1");
    assert_eq!(detail.status, CloudRunServiceStatus::Ready);
    assert_eq!(
        detail.ingress.as_deref(),
        Some("all"),
        "INGRESS_TRAFFIC_ALL is shown in the word the CLI pane prints, not as \
         the enum spells it"
    );
    assert_eq!(
        detail.image.as_deref(),
        Some("europe-docker.pkg.dev/example/apps/orders-api:1.4.2")
    );
    assert_eq!(detail.cpu.as_deref(), Some("1000m"));
    assert_eq!(detail.memory.as_deref(), Some("512Mi"));
    assert_eq!(detail.concurrency, Some(80));
    assert_eq!(detail.min_instances, Some(2));
    assert_eq!(detail.max_instances, Some(20));
    assert_eq!(
        detail.service_account.as_deref(),
        Some("orders-api@example.iam.gserviceaccount.com")
    );

    let mut sorted_labels = detail.labels.clone();
    sorted_labels.sort_by(|a, b| a.key.cmp(&b.key));
    assert_eq!(
        labels(&sorted_labels),
        [("env", "prod"), ("team", "commerce")],
        "both labels, VALUE included: the Labels row paints the value, and a \
         key-only assertion cannot tell `team=commerce` from any other string. \
         Sorted for the assertion because the model carries them in a `HashMap`"
    );

    assert_eq!(
        detail
            .traffic
            .iter()
            .map(|target| (
                target.revision_name.as_deref(),
                target.tag.as_deref(),
                target.percent,
                target.latest,
                target.url.as_deref()
            ))
            .collect::<Vec<_>>(),
        [
            (Some("orders-api-00007-tabc"), None, Some(100), true, None),
            (
                Some("orders-api-00006-sxyz"),
                Some("previous"),
                Some(0),
                false,
                Some("https://previous---orders-api-abcde-ew.a.run.app")
            ),
        ],
        "both slices of the split, with `latest` read off the allocation TYPE \
         rather than off an empty revision name"
    );

    assert_eq!(
        detail
            .conditions
            .iter()
            .map(|condition| (
                condition.type_.as_str(),
                condition.status.as_str(),
                condition.last_transition_time.as_deref()
            ))
            .collect::<Vec<_>>(),
        [
            ("Ready", "True", Some("2026-07-18T09:12:44Z")),
            ("ConfigurationsReady", "True", None),
            ("RoutesReady", "True", None),
        ],
        "the terminal condition first and then the rest: v2 moved readiness \
         out of `conditions` into its own field, so feeding only `conditions` \
         would drop the one line that says whether the service is serving"
    );

    assert_eq!(
        detail
            .revisions
            .iter()
            .map(|revision| (
                revision.name.as_str(),
                revision.ready.clone(),
                revision.active_percent
            ))
            .collect::<Vec<_>>(),
        [
            (
                "orders-api-00007-tabc",
                CloudRunServiceStatus::Ready,
                Some(100)
            ),
            (
                "orders-api-00006-sxyz",
                CloudRunServiceStatus::Unknown(String::new()),
                Some(0)
            ),
        ],
        "the split's two targets, derived into revision rows as the CLI pane \
         derives them: the latest ready revision marked Ready and the other \
         left unknown until the pane's own revisions fetch reports its true \
         health"
    );

    assert!(!detail.raw_json.is_empty(), "the pane has a JSON tab");
}

/// Every cell of every revision row.
#[tokio::test(flavor = "multi_thread")]
async fn the_fixture_cloud_run_revisions_carry_every_column_the_table_paints() {
    let rows = nuvem::services::sdk::cloud_run::SdkCloudRunService::from_fixtures()
        .load_revisions(PROJECT, "europe-west1", "orders-api")
        .await
        .expect("the fixture client answers");

    assert_eq!(
        rows.iter()
            .map(|row| (
                row.name.as_str(),
                row.ready.clone(),
                row.created.as_deref(),
                row.image.as_deref(),
                row.active_percent
            ))
            .collect::<Vec<_>>(),
        [
            (
                "orders-api-00007-tabc",
                CloudRunServiceStatus::Ready,
                Some("2026-07-18T09:11:58Z"),
                Some("europe-docker.pkg.dev/example/apps/orders-api:1.4.2"),
                None
            ),
            (
                "orders-api-00006-sxyz",
                CloudRunServiceStatus::Failed,
                Some("2026-07-10T14:05:22Z"),
                Some("europe-docker.pkg.dev/example/apps/orders-api:1.4.1"),
                None
            ),
        ],
        "both revisions. A revision reports readiness in its condition LIST -- \
         it has no terminal condition -- and `active_percent` stays unset here \
         because the split belongs to the service"
    );
    assert_eq!(
        rows[0].created.as_deref(),
        Some("2026-07-18T09:11:58Z"),
        "the create time round-trips through the protobuf `Timestamp` only \
         because the capture states a whole second in UTC; a sub-second part \
         or an offset would render differently from the string the CLI path \
         carries through"
    );
}

/// Every field of the revision detail pane.
#[tokio::test(flavor = "multi_thread")]
async fn the_fixture_cloud_run_revision_detail_carries_every_field_the_pane_paints() {
    let detail = nuvem::services::sdk::cloud_run::SdkCloudRunService::from_fixtures()
        .load_revision_detail(PROJECT, "europe-west1", "orders-api-00007-tabc")
        .await
        .expect("the fixture client answers");

    assert_eq!(detail.name, "orders-api-00007-tabc");
    assert_eq!(detail.region, "europe-west1");
    assert_eq!(
        detail.service.as_deref(),
        Some("orders-api"),
        "the owning service, shortened out of the `service` resource path -- \
         where the CLI path reads it from a Knative label"
    );
    assert_eq!(detail.ready, CloudRunServiceStatus::Ready);
    assert_eq!(
        detail
            .conditions
            .iter()
            .map(|condition| (condition.type_.as_str(), condition.status.as_str()))
            .collect::<Vec<_>>(),
        [
            ("Ready", "True"),
            ("Active", "True"),
            ("ContainerHealthy", "True"),
            ("ResourcesAvailable", "True"),
        ],
        "all four conditions, in the capture's order, each translated into the \
         True/False/Unknown vocabulary the pane already prints"
    );
    assert_eq!(detail.cpu.as_deref(), Some("1000m"));
    assert_eq!(detail.memory.as_deref(), Some("512Mi"));
    assert_eq!(detail.concurrency, Some(80));
    assert_eq!(detail.min_instances, Some(2));
    assert_eq!(detail.max_instances, Some(20));
    assert_eq!(
        detail.service_account.as_deref(),
        Some("orders-api@example.iam.gserviceaccount.com")
    );
    assert_eq!(detail.created.as_deref(), Some("2026-07-18T09:11:58Z"));
    assert!(!detail.raw_json.is_empty(), "the pane has a JSON tab");
}

/// The Cloud Run fixture rows and the CLI rows, cell for cell — and the three
/// cells where the two vocabularies cannot meet, pinned rather than fudged.
///
/// This is the comparison the re-shaping exists for. Cloud Run is the one
/// service whose native capture is a different document from the CLI's rather
/// than the same bytes in an envelope, so nothing else here can catch a v2
/// capture that drifted away from the Knative one it mirrors: a native-only
/// assertion written from the drifted file would agree with it.
#[tokio::test(flavor = "multi_thread")]
async fn the_fixture_cloud_run_rows_match_the_cli_rows_parsed_from_the_same_facts() {
    use nuvem::services::resources::cloud_run::{
        parse_revision_detail_json, parse_revisions_json, parse_service_detail_json,
        parse_services_json,
    };

    let service = nuvem::services::sdk::cloud_run::SdkCloudRunService::from_fixtures();

    let native = service.load_services(PROJECT).await.expect("answers");
    let cli = parse_services_json(CLOUD_RUN_SERVICES_JSON).expect("the capture parses");

    assert_eq!(native.len(), cli.len(), "the same five services");
    for (native_row, cli_row) in native.iter().zip(cli.iter()) {
        let at = cli_row.name.as_str();
        assert_eq!(native_row.name, cli_row.name, "{at}: name");
        assert_eq!(native_row.region, cli_row.region, "{at}: region");
        assert_eq!(native_row.url, cli_row.url, "{at}: url");
        assert_eq!(
            native_row.latest_ready_revision, cli_row.latest_ready_revision,
            "{at}: latest ready revision"
        );
        assert_eq!(
            native_row.latest_created_revision, cli_row.latest_created_revision,
            "{at}: latest created revision"
        );
        assert_eq!(
            native_row.traffic_summary, cli_row.traffic_summary,
            "{at}: traffic cell"
        );
        assert_eq!(
            native_row.ready_message, cli_row.ready_message,
            "{at}: ready message"
        );
    }

    for index in [0usize, 2, 3] {
        assert_eq!(
            native[index].status, cli[index].status,
            "{}: status",
            cli[index].name
        );
    }

    assert_eq!(
        (&native[1].status, &cli[1].status),
        (
            &CloudRunServiceStatus::NotReady,
            &CloudRunServiceStatus::NotReady
        ),
        "CLOSED: billing-worker read \"Failed\" toned Degraded natively and \
         \"Not Ready\" toned Warning on the CLI path. Knative distinguishes a \
         Ready=False WITH a reason (Failed) from one without (NotReady); v2 \
         folds both into CONDITION_FAILED but keeps the distinction in the \
         `reasons` one-of, and `status_from_state` now reads it. This capture \
         states no `revisionReason` on that condition precisely because the \
         original states no `reason`, so it is the not-yet-ready one on both \
         paths. checkout-api, two rows down, states one and reads Failed on \
         both -- the loop above covers it"
    );

    assert_eq!(
        (&native[4].status, &cli[4].status),
        (
            &CloudRunServiceStatus::Unknown("Unknown".to_string()),
            &CloudRunServiceStatus::Unknown("Unknown".to_string())
        ),
        "CLOSED: legacy-frontend states a Ready condition with an indefinite \
         status and no reason. This was pinned as an accepted divergence on \
         the argument that nothing visible moved, because \
         `cloud_run_status_label` answers \"Unknown\" for `Unknown(_)`. That \
         argument was wrong about the table: `project_resource_list` paints \
         the STATUS cell as `format!(\"{{:?}}\", status)`, payload included, so \
         `Unknown(\"\")` and `Unknown(\"Unknown\")` are two different cells -- \
         which is what `every_service_fills_its_painted_columns_through_a_demo\
         _session` caught the moment `--demo` moved to this backend. Two \
         things closed it. The v2 capture had dropped the readiness fact its \
         Knative original states, alone among the five rows, and now mirrors \
         it as `STATE_UNSPECIFIED`; and `status_from_state` runs an unmapped \
         state through `condition_status`, the translation this module already \
         applies to the Conditions pane so a row reads the same whichever \
         backend filled it"
    );

    let mut native = service
        .load_service_detail(PROJECT, "europe-west1", "orders-api")
        .await
        .expect("answers");
    let mut cli =
        parse_service_detail_json(CLOUD_RUN_SERVICE_DETAIL_JSON).expect("the capture parses");

    assert!(
        !native.raw_json.is_empty(),
        "the native pane has a JSON tab"
    );
    assert!(!cli.raw_json.is_empty(), "so does the CLI pane");
    native.raw_json.clear();
    cli.raw_json.clear();
    // Both sides build labels out of a `HashMap`; sorted so the comparison
    // measures content rather than the hasher.
    native.labels.sort_by(|a, b| a.key.cmp(&b.key));
    cli.labels.sort_by(|a, b| a.key.cmp(&b.key));

    let native_revisions = std::mem::take(&mut native.revisions);
    let cli_revisions = std::mem::take(&mut cli.revisions);
    let native_labels = std::mem::take(&mut native.labels);
    let cli_labels = std::mem::take(&mut cli.labels);

    assert_eq!(
        native, cli,
        "every other field of the service detail pane, including both traffic \
         slices and all three conditions"
    );

    assert_eq!(
        native_revisions, cli_revisions,
        "CLOSED: the CLI pane has always filled `revisions` from the traffic \
         split (`derive_revisions_from_status`), naming each target with the \
         split's percentage and marking the latest-ready one Ready; the native \
         pane left it empty and its doc comment claimed the CLI path did the \
         same. Both panes end up showing the fetched list, so what differed \
         was the instant before it arrives -- and that instant showed an \
         operator strictly less natively. `revisions_from_traffic` is the same \
         derivation over v2's `trafficStatuses`; the two lists are compared \
         here rather than each pinned to its own literal, because they are \
         separate implementations and a mutant in either moves only one side"
    );

    // The key SETS diverge by exactly one system label, pinned just below.
    // Every key both panes DO carry still has to agree on its VALUE: lifting
    // the labels out of the struct equality above is what lets that
    // divergence be pinned, and comparing only the key LIST took the values
    // out of the comparison along with it -- a wrong value in either capture
    // then painted a wrong Labels row with the whole suite green.
    let native_by_key: BTreeMap<&str, &str> = labels(&native_labels).into_iter().collect();
    let cli_by_key: BTreeMap<&str, &str> = labels(&cli_labels).into_iter().collect();
    for key in ["env", "team"] {
        assert_eq!(
            (key, native_by_key.get(key).copied()),
            (key, cli_by_key.get(key).copied()),
            "label `{key}`: both panes read it off the same service, so the \
             value the Labels row paints has to agree. A key one side drops \
             reads as `None` here rather than shortening a list"
        );
    }

    assert_eq!(
        (
            native_labels
                .iter()
                .map(|pair| pair.key.as_str())
                .collect::<Vec<_>>(),
            cli_labels
                .iter()
                .map(|pair| pair.key.as_str())
                .collect::<Vec<_>>()
        ),
        (
            vec!["env", "team"],
            vec!["cloud.googleapis.com/location", "env", "team"]
        ),
        "DIVERGENCE: the CLI path copies every Knative metadata label into the \
         pane, including the system label that carries the location; v2 has no \
         such label -- the location lives in the resource path, and the API \
         rejects labels in the `cloud.googleapis.com` namespace outright -- so \
         the native pane shows one row fewer. Writing the label into the v2 \
         capture would state something the API cannot return"
    );

    let native = service
        .load_revisions(PROJECT, "europe-west1", "orders-api")
        .await
        .expect("answers");
    assert_eq!(
        native,
        parse_revisions_json(CLOUD_RUN_REVISIONS_JSON).expect("the capture parses"),
        "the revision table agrees cell for cell on both backends"
    );

    let mut native = service
        .load_revision_detail(PROJECT, "europe-west1", "orders-api-00007-tabc")
        .await
        .expect("answers");
    let mut cli =
        parse_revision_detail_json(CLOUD_RUN_REVISION_DETAIL_JSON).expect("the capture parses");
    assert!(
        !native.raw_json.is_empty(),
        "the native pane has a JSON tab"
    );
    assert!(!cli.raw_json.is_empty(), "so does the CLI pane");
    native.raw_json.clear();
    cli.raw_json.clear();
    assert_eq!(
        native, cli,
        "and the revision detail pane agrees field for field, conditions \
         included -- a revision has no terminal condition, so nothing is \
         prepended to its list on either path"
    );
}

/// The hazard the re-shaping exists to avoid, reproduced on purpose.
///
/// An envelope over the UNTOUCHED Knative capture is the shortest thing that
/// looks like it works, and it is what every other service in this backend
/// genuinely does. Here it deserializes, it answers the right NUMBER of rows,
/// and every cell in them is blank: `ListServicesResponse` has no
/// `apiVersion`, `metadata`, `spec` or `status`, so each one is absorbed into
/// the private `_unknown_fields` map rather than rejected, and every modelled
/// field keeps its default.
///
/// A row-count assertion passes on this. The table paints five services with
/// no name, no region, no URL and a `-` traffic cell -- which reads as a
/// project whose services are all broken rather than as a fixture that is the
/// wrong shape.
#[test]
fn an_unshaped_knative_capture_fails_on_the_cells_rather_than_the_row_count() {
    use google_cloud_run_v2::model::ListServicesResponse;
    use nuvem::services::demo::sdk::envelope;
    use nuvem::services::sdk::cloud_run::map_service;

    let response: ListServicesResponse = envelope("services", CLOUD_RUN_SERVICES_JSON)
        .expect("the Knative capture is valid JSON, so the envelope succeeds");
    let rows: Vec<_> = response.services.iter().map(map_service).collect();

    assert_eq!(
        rows.len(),
        5,
        "the row COUNT survives the wrong vocabulary, which is why no count \
         assertion can guard this"
    );
    assert!(
        rows.iter().all(|row| row.name.is_empty()
            && row.region.is_empty()
            && row.url.is_none()
            && row.latest_ready_revision.is_none()
            && row.traffic_summary == "-"
            && row.status == CloudRunServiceStatus::Unknown(String::new())),
        "every cell is blank: {rows:?}"
    );
}

// ---------------------------------------------------------------------------
// Cloud Storage
// ---------------------------------------------------------------------------

const BUCKETS_JSON: &str = include_str!("fixtures/gcs_buckets.json");
const BUCKET_DETAIL_JSON: &str = include_str!("fixtures/gcs_bucket_detail.json");
const OBJECTS_JSON: &str = include_str!("fixtures/gcs_objects.json");
const OBJECT_DETAIL_JSON: &str = include_str!("fixtures/gcs_object_detail.json");

/// Every cell of every bucket row, including the two that live one message
/// deep in v2 where gcloud printed them flat.
#[tokio::test(flavor = "multi_thread")]
async fn the_fixture_buckets_carry_every_column_the_table_paints() {
    let rows = nuvem::services::sdk::storage::SdkStorageService::from_fixtures()
        .load_buckets(PROJECT)
        .await
        .expect("the fixture client answers");

    assert_eq!(
        rows.iter().map(|row| row.name.as_str()).collect::<Vec<_>>(),
        ["alpha-bucket", "beta-bucket", "gamma-bucket"],
        "the capture's three buckets, each named by `bucketId` rather than by \
         the `projects/_/buckets/<id>` resource name"
    );

    assert_eq!(rows[0].location, "US", "alpha: location");
    assert_eq!(
        rows[0].location_type.as_deref(),
        Some("multi-region"),
        "alpha: location type"
    );
    assert_eq!(
        rows[0].storage_class, "STANDARD",
        "alpha: storage class -- `storageClass`, where gcloud prints \
         `default_storage_class`"
    );
    assert!(
        rows[0].versioning_enabled,
        "alpha: versioning, read out of `versioning.enabled` where gcloud \
         prints a flat `versioning_enabled`"
    );
    assert!(
        rows[0].uniform_bucket_level_access,
        "alpha: uniform access, read out of \
         `iamConfig.uniformBucketLevelAccess.enabled`"
    );
    assert_eq!(
        rows[0].public_access_prevention.as_deref(),
        Some("enforced"),
        "alpha: public access prevention, read out of `iamConfig`"
    );
    assert_eq!(
        rows[0].time_created.as_deref(),
        Some("2024-01-02T03:04:05Z"),
        "alpha: created"
    );
    assert_eq!(
        rows[0].updated.as_deref(),
        Some("2024-02-03T04:05:06Z"),
        "alpha: updated"
    );

    assert!(
        !rows[1].versioning_enabled && !rows[1].uniform_bucket_level_access,
        "beta: both settings are present and false, not absent"
    );
    assert_eq!(
        rows[1].updated, None,
        "beta: a bucket that has never been updated shows nothing"
    );

    assert_eq!(
        (
            rows[2].location_type.as_deref(),
            rows[2].public_access_prevention.as_deref(),
            rows[2].versioning_enabled,
            rows[2].uniform_bucket_level_access,
        ),
        (None, None, false, false),
        "gamma states neither `iamConfig` nor `versioning`, and a missing \
         message reads as absent or false rather than as an empty string"
    );
}

/// One level of a bucket, which is what the browser shows: the folders
/// directly under the prefix and the objects sitting at it.
#[tokio::test(flavor = "multi_thread")]
async fn the_fixture_object_page_describes_the_level_the_request_asks_for() {
    let service = nuvem::services::sdk::storage::SdkStorageService::from_fixtures();

    let root = service
        .load_object_page("alpha-bucket", "", None, 100)
        .await
        .expect("the fixture client answers");
    assert_eq!(
        root.prefixes
            .iter()
            .map(|prefix| prefix.name.as_str())
            .collect::<Vec<_>>(),
        ["docs/", "photos/", "reports/"],
        "the three folders at the root, which the delimiter is what produces: \
         the capture is a flat recursive listing and names no folder anywhere"
    );
    assert_eq!(
        root.objects
            .iter()
            .map(|object| object.key.as_str())
            .collect::<Vec<_>>(),
        [
            "top-level.txt",
            "name.bin",
            "temp-hold.txt",
            "event-hold.txt",
            "retained.txt"
        ],
        "and only the objects at the root, in the capture's order"
    );
    assert_eq!(
        root.next_page_token, None,
        "one page: the stub issues no token, so the browser does not ask for a \
         second"
    );

    let photos = service
        .load_object_page("alpha-bucket", "photos/", None, 100)
        .await
        .expect("the fixture client answers");
    assert_eq!(
        (
            photos
                .prefixes
                .iter()
                .map(|prefix| prefix.name.as_str())
                .collect::<Vec<_>>(),
            photos
                .objects
                .iter()
                .map(|object| object.key.as_str())
                .collect::<Vec<_>>()
        ),
        (vec![], vec!["photos/img1.jpg", "photos/img2.jpg"]),
        "opening a folder answers THAT folder. A single canned page would have \
         answered the root listing here, painting `top-level.txt` inside \
         `photos/` -- which is why the stub groups off the request's own prefix \
         and delimiter rather than serving one document"
    );

    let nested = service
        .load_object_page("alpha-bucket", "reports/", None, 100)
        .await
        .expect("the fixture client answers");
    assert_eq!(
        (
            nested
                .prefixes
                .iter()
                .map(|prefix| prefix.name.as_str())
                .collect::<Vec<_>>(),
            nested.objects.len()
        ),
        (vec!["reports/2024/"], 0),
        "a folder whose contents are all one level deeper lists as one folder \
         and no objects"
    );

    let cells = &root.objects[1];
    assert_eq!(cells.key, "name.bin");
    assert_eq!(cells.size_bytes, Some(256), "size");
    assert_eq!(cells.storage_class.as_deref(), Some("STANDARD"));
    assert_eq!(
        cells.generation.as_deref(),
        Some("1700000000000001"),
        "the generation is an int64 in v2 and a decimal string in gcloud's \
         output; the cell is the same string either way"
    );
    assert_eq!(
        cells.content_type.as_deref(),
        Some("application/octet-stream")
    );
    assert!(
        cells.kms_encrypted,
        "only the PRESENCE of the key is carried, never its name"
    );
    assert!(
        root.objects[2].temporary_hold,
        "temp-hold.txt: temporary hold"
    );
    assert!(
        root.objects[3].event_based_hold,
        "event-hold.txt: event-based hold"
    );
    assert!(
        root.objects[4].has_retention,
        "retained.txt: a retention expiry is present"
    );
}

/// Every field of the bucket detail pane.
#[tokio::test(flavor = "multi_thread")]
async fn the_fixture_bucket_detail_carries_every_field_the_pane_paints() {
    let detail = nuvem::services::sdk::storage::SdkStorageService::from_fixtures()
        .load_bucket_detail("alpha-bucket")
        .await
        .expect("the fixture client answers");

    assert_eq!(detail.name, "alpha-bucket");
    assert_eq!(detail.location, "US");
    assert_eq!(detail.storage_class, "STANDARD");
    assert!(detail.versioning_enabled);
    assert!(detail.uniform_bucket_level_access);
    assert_eq!(detail.public_access_prevention.as_deref(), Some("enforced"));
    assert!(
        detail.requester_pays,
        "read out of `billing.requesterPays`, which decides who is charged for \
         a read and makes an un-opted-in listing fail outright"
    );
    assert_eq!(
        detail.retention_period.as_deref(),
        Some("2592000"),
        "the CLI reads a seconds COUNT as a string and v2 models a Duration, \
         so `2592000s` has to come back out as the same digits the CLI pane \
         prints"
    );
    assert!(
        detail.default_kms_key_present,
        "presence only -- the key's name must not reach the domain layer"
    );
    let mut sorted = detail.labels.clone();
    sorted.sort_by(|a, b| a.key.cmp(&b.key));
    assert_eq!(
        sorted
            .iter()
            .map(|pair| (pair.key.as_str(), pair.value.as_str()))
            .collect::<Vec<_>>(),
        [("env", "prod"), ("team", "platform")],
        "both labels; sorted for the assertion because the model carries them \
         in a `HashMap`"
    );
    assert!(
        !detail.raw_json.contains("gcs-ring"),
        "the JSON tab must not carry the KMS key name either: {}",
        detail.raw_json
    );
    assert!(
        detail.raw_json.contains("<redacted>"),
        "and it says so rather than dropping the field"
    );
}

/// Every field of the object detail pane, including the two checksums that are
/// the one place the re-shaping changes units rather than names.
#[tokio::test(flavor = "multi_thread")]
async fn the_fixture_object_detail_carries_every_field_the_pane_paints() {
    let detail = nuvem::services::sdk::storage::SdkStorageService::from_fixtures()
        .load_object_detail("alpha-bucket", "reports/2024/mañana-测试.txt")
        .await
        .expect("the fixture client answers");

    assert_eq!(detail.key, "reports/2024/mañana-测试.txt");
    assert_eq!(detail.size_bytes, Some(1024));
    assert_eq!(detail.storage_class.as_deref(), Some("STANDARD"));
    assert_eq!(detail.generation.as_deref(), Some("1700000000000001"));
    assert_eq!(detail.updated.as_deref(), Some("2024-03-01T10:00:00Z"));
    assert_eq!(detail.content_type.as_deref(), Some("text/plain"));
    assert!(detail.kms_encrypted);
    assert!(!detail.temporary_hold);
    assert!(!detail.event_based_hold);
    assert_eq!(
        detail.retention_expiration.as_deref(),
        Some("2025-06-01T00:00:00Z")
    );
    assert_eq!(
        detail.md5_hash.as_deref(),
        Some("XUFAKrxLKna5cZ2REBfFkg=="),
        "protobuf `bytes` is base64 in JSON, so the MD5 needs no conversion -- \
         it is the same string gcloud prints"
    );
    assert_eq!(
        detail.crc32c.as_deref(),
        Some("AAAAAA=="),
        "the CRC does: `ObjectChecksums.crc32c` is a fixed32, so the capture \
         states the NUMBER `AAAAAA==` encodes and the mapper encodes it back. \
         That number is zero, and writing the zero out is load-bearing -- the \
         field is an Option, so omitting it blanks this line where the CLI \
         pane prints a hash"
    );
    assert!(
        !detail.raw_json.contains("gcs-ring"),
        "the JSON tab must not carry the KMS key name: {}",
        detail.raw_json
    );
}

/// The Cloud Storage fixture rows and the CLI rows, cell for cell.
#[tokio::test(flavor = "multi_thread")]
async fn the_fixture_storage_rows_match_the_cli_rows_parsed_from_the_same_facts() {
    use nuvem::services::resources::storage::{
        parse_bucket_detail_json, parse_buckets_json, parse_object_detail_json,
        parse_object_page_json,
    };

    let service = nuvem::services::sdk::storage::SdkStorageService::from_fixtures();

    assert_eq!(
        service.load_buckets(PROJECT).await.expect("answers"),
        parse_buckets_json(BUCKETS_JSON).expect("the capture parses"),
        "the two backends must paint the same buckets table, one reading \
         gcloud's invented vocabulary and one reading Storage v2"
    );

    // Every level the capture has, not just the root: the grouping is the part
    // of this stub that could silently answer the wrong folder's contents.
    for prefix in ["", "photos/", "docs/", "reports/", "reports/2024/"] {
        assert_eq!(
            service
                .load_object_page("alpha-bucket", prefix, None, 100)
                .await
                .expect("answers"),
            parse_object_page_json(OBJECTS_JSON, prefix).expect("the capture parses"),
            "the same page at {prefix:?}: the stub groups by prefix and \
             delimiter where `parse_object_page_json` groups the flat listing"
        );
    }

    let mut native = service
        .load_bucket_detail("alpha-bucket")
        .await
        .expect("answers");
    let mut cli = parse_bucket_detail_json(BUCKET_DETAIL_JSON).expect("the capture parses");
    assert!(
        !native.raw_json.is_empty(),
        "the native pane has a JSON tab"
    );
    assert!(!cli.raw_json.is_empty(), "so does the CLI pane");
    native.raw_json.clear();
    cli.raw_json.clear();
    // Both sides build labels out of a map; sorted so the comparison measures
    // content rather than the hasher.
    native.labels.sort_by(|a, b| a.key.cmp(&b.key));
    cli.labels.sort_by(|a, b| a.key.cmp(&b.key));
    assert_eq!(native, cli, "and the same bucket detail, field for field");

    let mut native = service
        .load_object_detail("alpha-bucket", "reports/2024/mañana-测试.txt")
        .await
        .expect("answers");
    let mut cli = parse_object_detail_json(OBJECT_DETAIL_JSON).expect("the capture parses");
    native.raw_json.clear();
    cli.raw_json.clear();
    assert_eq!(
        native, cli,
        "and the same object detail, checksums included"
    );
}

// ---------------------------------------------------------------------------
// Project discovery
// ---------------------------------------------------------------------------

const PROJECTS_JSON: &str = include_str!("fixtures/gcloud_projects.json");

/// Every cell of every project row, and the cross-backend comparison beside
/// it.
///
/// The two are one test because the failure they guard is one failure. Reading
/// the v1 capture through `SearchProjectsResponse` does not error: `projectId`,
/// `name` and `lifecycleState` are all absorbed into `_unknown_fields`, and the
/// picker gets two rows with a blank id, a blank name and no state. A count
/// passes on that; these assertions do not.
#[tokio::test(flavor = "multi_thread")]
async fn the_fixture_projects_carry_every_column_the_picker_paints() {
    use nuvem::services::gcloud::discovery::parse_projects_json;

    let rows = nuvem::services::sdk::discovery::SdkDiscoveryService::from_fixtures()
        .load_projects()
        .await
        .expect("the fixture client answers");

    assert_eq!(
        rows.iter()
            .map(|row| (
                row.project_id.as_str(),
                row.name.as_str(),
                row.lifecycle_state.as_deref()
            ))
            .collect::<Vec<_>>(),
        [
            ("dev-project", "Development", Some("ACTIVE")),
            ("prod-project", "Production", Some("ACTIVE")),
        ],
        "both projects. v3 renames all three cells: the id is `projectId`, the \
         picker's NAME is `displayName` (v3's `name` is the resource path \
         `projects/<number>`), and the lifecycle is `state`, taken by name \
         rather than by its wire number so the picker's incremental filter \
         still matches what is painted"
    );

    assert_eq!(
        rows,
        parse_projects_json(PROJECTS_JSON).expect("the capture parses"),
        "and the two backends paint the same picker: the v3 capture mirrors \
         the v1 one row for row"
    );
}

// ---------------------------------------------------------------------------
// What the stubs read off the request
// ---------------------------------------------------------------------------

/// The half of a stub-backed read that nothing used to observe.
///
/// The wire-level seam is justified partly by "a stub-backed client still
/// re-runs the request builder", and it does -- but a request nobody reads
/// proves only that the builder did not panic. Measured before these checks
/// existed: `include_all_scopes(false)`, `return_partial_success(false)`,
/// Cloud Run's list parent set to `locations/PROBE` and Storage's to
/// `projects/PROBE-{project}` are each a live-API behaviour change or an
/// outright failure, and all four left the entire suite green.
///
/// They do not now. Each case below is one of those mutations, and each fails
/// here by name rather than as a blank cell somewhere downstream.
mod the_stubs_read_the_request_before_they_answer_it {
    use super::PROJECT;

    /// The offending value has to appear in the refusal, or the failure names
    /// nothing an author can act on.
    fn refusal_naming(error: &google_cloud_gax::error::Error, offender: &str) -> String {
        let rendered = format!("{error:?}");
        assert!(
            rendered.contains(offender),
            "a refusal must name what it refused; `{offender}` is not in {rendered}"
        );
        rendered
    }

    /// A project the demo corpus does not contain is refused, not answered.
    ///
    /// `PROBE-dev-project` is a syntactically perfect project id, which is the
    /// point: shape alone cannot separate it from the real one, so the check
    /// is against the project capture the picker itself lists.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_aggregated_listing_for_a_project_this_demo_has_never_heard_of_is_refused() {
        let unknown = format!("PROBE-{PROJECT}");

        let instances = nuvem::services::demo::sdk::compute::client()
            .aggregated_list()
            .set_project(&unknown)
            .set_include_all_scopes(true)
            .set_return_partial_success(true)
            .send()
            .await
            .expect_err("a project the fixtures do not hold has no listing");
        refusal_naming(&instances, &unknown);

        let disks = nuvem::services::demo::sdk::disks::client()
            .aggregated_list()
            .set_project(&unknown)
            .set_include_all_scopes(true)
            .set_return_partial_success(true)
            .send()
            .await
            .expect_err("and neither does its disk listing");
        refusal_naming(&disks, &unknown);

        let (_networks, subnetworks, _firewalls) = nuvem::services::demo::sdk::vpc::clients();
        let subnets = subnetworks
            .aggregated_list()
            .set_project(&unknown)
            .send()
            .await
            .expect_err("nor its subnets");
        refusal_naming(&subnets, &unknown);
    }

    /// Compute takes the project as a bare id. A caller that hands it the
    /// resource path every other API family wants builds a URL with two
    /// `projects/` segments, which is a 404 against Google and used to be a
    /// full table here.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_aggregated_listing_given_a_resource_path_instead_of_a_project_id_is_refused() {
        let path = format!("projects/{PROJECT}");
        let error = nuvem::services::demo::sdk::compute::client()
            .aggregated_list()
            .set_project(&path)
            .set_include_all_scopes(true)
            .set_return_partial_success(true)
            .send()
            .await
            .expect_err("`projects/dev-project` is not a Compute project id");
        refusal_naming(&error, &path);
    }

    /// Either scope flag turned off is a different set of scopes on the live
    /// API, so the stub will not answer as though it were the same request.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_aggregated_listing_with_a_scope_flag_turned_off_is_refused() {
        for (all_scopes, partial) in [(false, true), (true, false)] {
            let instances = nuvem::services::demo::sdk::compute::client()
                .aggregated_list()
                .set_project(PROJECT)
                .set_include_all_scopes(all_scopes)
                .set_return_partial_success(partial)
                .send()
                .await
                .expect_err("the flags gcloud sends are the flags this answers");
            refusal_naming(&instances, "true");

            let disks = nuvem::services::demo::sdk::disks::client()
                .aggregated_list()
                .set_project(PROJECT)
                .set_include_all_scopes(all_scopes)
                .set_return_partial_success(partial)
                .send()
                .await
                .expect_err("disks send the same two");
            refusal_naming(&disks, "true");
        }
    }

    /// A parent assembled from the wrong pieces is refused by the RPC it was
    /// built for, as the live API refuses it.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_parent_that_does_not_match_its_rpcs_template_is_refused() {
        let (services, revisions) = nuvem::services::demo::sdk::cloud_run::clients();

        let dropped_half = services
            .list_services()
            .set_parent("locations/PROBE")
            .send()
            .await
            .expect_err("a service listing is scoped to a project and a location");
        refusal_naming(&dropped_half, "locations/PROBE");

        // The service listing's own parent, one level too short for revisions.
        let too_short = revisions
            .list_revisions()
            .set_parent(format!("projects/{PROJECT}/locations/-"))
            .send()
            .await
            .expect_err("revisions hang off a service, not off a location");
        refusal_naming(&too_short, "projects/*/locations/*/services/*");

        let buckets = nuvem::services::demo::sdk::storage::client()
            .list_buckets()
            .set_parent(format!("projects/PROBE-{PROJECT}"))
            .send()
            .await
            .expect_err("a project the fixtures do not hold holds no buckets");
        refusal_naming(&buckets, &format!("PROBE-{PROJECT}"));

        let objects = nuvem::services::demo::sdk::storage::client()
            .list_objects()
            .set_parent("buckets/alpha-bucket")
            .send()
            .await
            .expect_err("an object listing's parent carries the `projects/_` prefix");
        refusal_naming(&objects, "buckets/alpha-bucket");
    }

    /// Both projects, because neither is the demo project.
    ///
    /// An operator can activate the `prod` configuration in `--demo`, and the
    /// context's project becomes `prod-project`. A check written against one
    /// hard-coded id would paint a refusal across every table the moment they
    /// did, which is why the set is read off the same capture the project
    /// picker lists rather than written down.
    #[tokio::test(flavor = "multi_thread")]
    async fn every_project_this_demo_universe_contains_is_answered() {
        for project in ["dev-project", "prod-project"] {
            let page = nuvem::services::demo::sdk::compute::client()
                .aggregated_list()
                .set_project(project)
                .set_include_all_scopes(true)
                .set_return_partial_success(true)
                .send()
                .await
                .unwrap_or_else(|error| panic!("{project} is a demo project: {error:?}"));
            assert!(
                !page.items.is_empty(),
                "{project} answers the capture, not an empty listing"
            );

            let buckets = nuvem::services::demo::sdk::storage::client()
                .list_buckets()
                .set_parent(format!("projects/{project}"))
                .send()
                .await
                .unwrap_or_else(|error| panic!("{project} is a demo project: {error:?}"));
            assert!(
                !buckets.buckets.is_empty(),
                "{project} answers the bucket capture too"
            );
        }
    }
}
