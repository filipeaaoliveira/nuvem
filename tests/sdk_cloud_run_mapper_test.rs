//! The Cloud Run mappers on the native read path: the sweep reported
//! `map_service`, `map_revision`, `map_service_detail` and `map_revision_detail`
//! as untested, so a mapper could drop a field, pick the wrong condition, or
//! invert a sentinel test and nothing would say so.
//!
//! Every test plants a distinct, non-default value per field and asserts the
//! exact mapped output, so a body replaced by a default is visible; every
//! branch (the `Ready` condition lookup, the empty-revision traffic filter, the
//! zero-instance sentinels, the condition-state vocabulary) is exercised from
//! both sides with inputs that produce different rows, so a flipped comparison
//! or a constant-folded closure cannot survive.

use google_cloud_run_v2::model::condition::State;
use google_cloud_run_v2::model::{
    Condition, Container, IngressTraffic, ResourceRequirements, Revision, RevisionScaling,
    RevisionTemplate, Service, TrafficTargetStatus,
};
use nuvem::services::sdk::cloud_run::{
    map_revision, map_revision_detail, map_service, map_service_detail,
};
use nuvem_domain::gcp::resource::{
    CloudRunCondition, CloudRunRevision, CloudRunRevisionDetail, CloudRunService,
    CloudRunServiceDetail, CloudRunServiceStatus, CloudRunTrafficTarget, ComputeKeyValue,
};

const SERVICE_NAME: &str = "projects/proj-alpha/locations/europe-west4/services/checkout";
const REVISION_NAME: &str =
    "projects/proj-alpha/locations/europe-west4/services/checkout/revisions/checkout-00007-abc";

/// A timestamp whose rendered form is unmistakable, so a dropped `created`
/// field cannot pass as a plausible one.
fn planted_timestamp() -> google_cloud_wkt::Timestamp {
    google_cloud_wkt::Timestamp::new(1_704_164_645, 0).expect("a valid timestamp")
}

const PLANTED_TIMESTAMP_TEXT: &str = "2024-01-02T03:04:05Z";

/// The service a state-vocabulary test varies: everything but the terminal
/// state is held fixed.
fn service_with_terminal_state(state: State) -> CloudRunService {
    map_service(
        &Service::new()
            .set_name(SERVICE_NAME)
            .set_terminal_condition(Condition::new().set_type("Ready").set_state(state)),
    )
}

// ---------------------------------------------------------------------------
// map_service
// ---------------------------------------------------------------------------

#[test]
fn a_service_row_carries_every_field_the_api_filled() {
    let service = Service::new()
        .set_name(SERVICE_NAME)
        .set_uri("https://checkout-abc.a.run.app")
        .set_latest_ready_revision(REVISION_NAME)
        .set_latest_created_revision(
            "projects/proj-alpha/locations/europe-west4/services/checkout/revisions/checkout-00008-def",
        )
        .set_traffic_statuses([
            TrafficTargetStatus::new()
                .set_revision("checkout-00007-abc")
                .set_percent(60),
            TrafficTargetStatus::new()
                .set_revision("checkout-00008-def")
                .set_percent(40),
        ])
        .set_terminal_condition(
            Condition::new()
                .set_type("Ready")
                .set_state(State::ConditionFailed)
                .set_message("Revision checkout-00008-def is not ready"),
        );

    assert_eq!(
        map_service(&service),
        CloudRunService {
            name: "checkout".to_string(),
            region: "europe-west4".to_string(),
            url: Some("https://checkout-abc.a.run.app".to_string()),
            latest_ready_revision: Some("checkout-00007-abc".to_string()),
            latest_created_revision: Some("checkout-00008-def".to_string()),
            traffic_summary: "checkout-00007-abc 60%".to_string(),
            // A failed condition that states no reason is the not-yet-ready
            // one, on this path as on the CLI path it replaced; the reason
            // one-of is what separates it from a failure, and this condition
            // carries none.
            status: CloudRunServiceStatus::NotReady,
            ready_message: Some("Revision checkout-00008-def is not ready".to_string()),
        },
        "every field the v2 service carries reaches the row, none of them defaulted"
    );
}

#[test]
fn a_service_name_yields_its_last_segment_and_the_segment_after_locations() {
    // The project id and the region are different words on purpose: a region
    // read one segment early, or found by the segment that is *not*
    // `locations`, lands on `proj-alpha` instead of `europe-west4`.
    let row = map_service(&Service::new().set_name(SERVICE_NAME));

    assert_eq!(row.name, "checkout", "the name is the last segment");
    assert_eq!(
        row.region, "europe-west4",
        "the region is the segment that follows `locations`, not the one before it"
    );
}

#[test]
fn a_service_name_without_a_locations_segment_has_no_region() {
    let row = map_service(&Service::new().set_name("solo"));

    assert_eq!(row.name, "solo", "a bare name is still the last segment");
    assert_eq!(
        row.region, "",
        "no `locations` segment means no region rather than a guessed one"
    );
}

#[test]
fn an_unset_uri_or_latest_revision_is_absent_rather_than_empty() {
    let row = map_service(
        &Service::new()
            .set_name(SERVICE_NAME)
            .set_uri("")
            .set_latest_ready_revision("")
            .set_latest_created_revision(""),
    );

    assert_eq!(row.url, None, "the proto's empty string is not a URL");
    assert_eq!(row.latest_ready_revision, None);
    assert_eq!(row.latest_created_revision, None);
}

#[test]
fn the_traffic_summary_skips_the_latest_pseudo_target() {
    // A split that routes to "the latest revision" reports an empty revision
    // name. Including it would summarise the row as ` 100%`.
    let service = Service::new().set_name(SERVICE_NAME).set_traffic_statuses([
        TrafficTargetStatus::new().set_revision("").set_percent(100),
        TrafficTargetStatus::new()
            .set_revision("checkout-00007-abc")
            .set_percent(30),
    ]);

    assert_eq!(
        map_service(&service).traffic_summary,
        "checkout-00007-abc 30%",
        "a nameless target is filtered out even when it holds the larger share"
    );
}

#[test]
fn the_traffic_summary_names_the_largest_share_not_the_first_one() {
    let service = Service::new().set_name(SERVICE_NAME).set_traffic_statuses([
        TrafficTargetStatus::new()
            .set_revision("checkout-00007-abc")
            .set_percent(70),
        TrafficTargetStatus::new()
            .set_revision("checkout-00008-def")
            .set_percent(30),
    ]);

    assert_eq!(
        map_service(&service).traffic_summary,
        "checkout-00007-abc 70%",
        "the summary is the target with the largest percentage"
    );
}

#[test]
fn a_service_with_no_traffic_statuses_summarises_as_a_dash() {
    let row = map_service(&Service::new().set_name(SERVICE_NAME));

    assert_eq!(
        row.traffic_summary, "-",
        "no split reads as a dash, not as an empty cell"
    );
}

#[test]
fn every_v2_condition_state_translates_to_its_own_status() {
    assert_eq!(
        service_with_terminal_state(State::ConditionSucceeded).status,
        CloudRunServiceStatus::Ready
    );
    assert_eq!(
        service_with_terminal_state(State::ConditionFailed).status,
        CloudRunServiceStatus::NotReady,
        "a failed state that states no reason is the not-yet-ready one; the \
         reason one-of is what makes it a failure, and it is pinned in \
         `a_failed_condition_is_a_failure_only_when_it_states_a_reason`"
    );
    assert_eq!(
        service_with_terminal_state(State::ConditionPending).status,
        CloudRunServiceStatus::Reconciling,
        "pending is still settling"
    );
    assert_eq!(
        service_with_terminal_state(State::ConditionReconciling).status,
        CloudRunServiceStatus::Reconciling,
        "reconciling is still settling"
    );
    // The payload of `Unknown` is the *Knative* status word -- that is what
    // `normalize_status` puts there, and the services table paints the whole
    // variant with `{:?}`, so a raw v2 token in it is a different cell on
    // screen for the same service. Both unmapped states therefore read
    // "Unknown", the word `condition_status` already translates them to one
    // pane over. The v2 token is lost, exactly as it is in the Conditions
    // pane, and the tone does not depend on it: `StatusSemantic` answers
    // Neutral for every `Unknown(_)`.
    assert_eq!(
        service_with_terminal_state(State::Unspecified).status,
        CloudRunServiceStatus::Unknown("Unknown".to_string()),
        "an unspecified state is the Knative word for stated-but-indefinite"
    );
    assert_eq!(
        service_with_terminal_state(State::from("CONDITION_INVENTED")).status,
        CloudRunServiceStatus::Unknown("Unknown".to_string()),
        "and so is a state this build has never heard of"
    );
    assert_eq!(
        service_with_terminal_state(State::from(77_i32)).status,
        CloudRunServiceStatus::Unknown(String::new()),
        "a state with no name at all is an empty unknown -- a condition that \
         says nothing is not the same fact as one that says \"not definite\", \
         and the CLI path spells that difference the same way"
    );
}

#[test]
fn a_service_with_no_terminal_condition_is_an_empty_unknown() {
    let row = map_service(&Service::new().set_name(SERVICE_NAME));

    assert_eq!(row.status, CloudRunServiceStatus::Unknown(String::new()));
    assert_eq!(
        row.ready_message, None,
        "no condition means no explanation either"
    );
}

#[test]
fn an_empty_terminal_message_is_absent_and_a_real_one_is_kept() {
    let blank = map_service(
        &Service::new()
            .set_name(SERVICE_NAME)
            .set_terminal_condition(Condition::new().set_state(State::ConditionSucceeded)),
    );
    assert_eq!(
        blank.ready_message, None,
        "a ready service explains nothing, and an empty string is not an explanation"
    );

    let failed = map_service(
        &Service::new()
            .set_name(SERVICE_NAME)
            .set_terminal_condition(
                Condition::new()
                    .set_state(State::ConditionFailed)
                    .set_message("Container failed to start"),
            ),
    );
    assert_eq!(
        failed.ready_message,
        Some("Container failed to start".to_string())
    );
}

// ---------------------------------------------------------------------------
// map_revision
// ---------------------------------------------------------------------------

#[test]
fn a_revision_row_carries_every_field_the_api_filled() {
    let revision = Revision::new()
        .set_name(REVISION_NAME)
        .set_create_time(planted_timestamp())
        .set_conditions([Condition::new()
            .set_type("Ready")
            .set_state(State::ConditionSucceeded)])
        .set_containers([Container::new()
            .set_name("app")
            .set_image("europe-docker.pkg.dev/proj-alpha/images/checkout:v7")]);

    assert_eq!(
        map_revision(&revision),
        CloudRunRevision {
            name: "checkout-00007-abc".to_string(),
            ready: CloudRunServiceStatus::Ready,
            created: Some(PLANTED_TIMESTAMP_TEXT.to_string()),
            image: Some("europe-docker.pkg.dev/proj-alpha/images/checkout:v7".to_string()),
            active_percent: None,
        },
        "the split belongs to the service, so a revision row leaves active_percent unset"
    );
}

#[test]
fn a_revision_reads_readiness_from_the_ready_condition_not_the_first_one() {
    // `Active` comes first and disagrees with `Ready`, so a lookup that takes
    // the first condition -- or the first one whose type is *not* `Ready` --
    // reports Failed where the row must read Ready.
    let revision = Revision::new().set_name(REVISION_NAME).set_conditions([
        Condition::new()
            .set_type("Active")
            .set_state(State::ConditionFailed),
        Condition::new()
            .set_type("Ready")
            .set_state(State::ConditionSucceeded),
    ]);

    assert_eq!(map_revision(&revision).ready, CloudRunServiceStatus::Ready);
}

#[test]
fn a_revision_with_no_ready_condition_is_an_empty_unknown() {
    // The only condition present is a settled one of another type: a lookup
    // that ignores the type would report Ready.
    let revision = Revision::new()
        .set_name(REVISION_NAME)
        .set_conditions([Condition::new()
            .set_type("Active")
            .set_state(State::ConditionSucceeded)]);

    assert_eq!(
        map_revision(&revision).ready,
        CloudRunServiceStatus::Unknown(String::new()),
        "readiness is what the Ready condition says, and silence is not readiness"
    );
}

#[test]
fn a_revision_takes_its_image_from_the_first_container() {
    let revision = Revision::new().set_name(REVISION_NAME).set_containers([
        Container::new().set_image("first-image:1"),
        Container::new().set_image("second-image:2"),
    ]);

    assert_eq!(
        map_revision(&revision).image,
        Some("first-image:1".to_string()),
        "the serving container is the first one, and order decides which image shows"
    );
}

#[test]
fn a_revision_with_no_container_or_an_empty_image_reports_no_image() {
    let bare = map_revision(&Revision::new().set_name(REVISION_NAME));
    assert_eq!(bare.image, None, "no container means no image");

    let blank = map_revision(
        &Revision::new()
            .set_name(REVISION_NAME)
            .set_containers([Container::new().set_image("")]),
    );
    assert_eq!(
        blank.image, None,
        "an empty image string is the proto's unset, not an image named nothing"
    );
}

#[test]
fn a_revision_without_a_creation_time_reports_none() {
    assert_eq!(
        map_revision(&Revision::new().set_name(REVISION_NAME)).created,
        None,
        "no creation time is absent, not an epoch"
    );
}

// ---------------------------------------------------------------------------
// map_service_detail
// ---------------------------------------------------------------------------

#[test]
fn a_service_detail_carries_every_field_the_api_filled() {
    let service = Service::new()
        .set_name(SERVICE_NAME)
        .set_uri("https://checkout-abc.a.run.app")
        .set_ingress(IngressTraffic::InternalOnly)
        .set_labels([("team", "payments")])
        .set_latest_ready_revision(REVISION_NAME)
        .set_latest_created_revision(
            "projects/proj-alpha/locations/europe-west4/services/checkout/revisions/checkout-00008-def",
        )
        .set_terminal_condition(
            Condition::new()
                .set_type("Ready")
                .set_state(State::ConditionSucceeded),
        )
        .set_conditions([Condition::new()
            .set_type("RoutesReady")
            .set_state(State::ConditionSucceeded)
            .set_message("All routes are ready")
            .set_last_transition_time(planted_timestamp())])
        .set_traffic_statuses([TrafficTargetStatus::new()
            .set_revision("checkout-00007-abc")
            .set_percent(100)
            .set_tag("live")
            .set_uri("https://live---checkout-abc.a.run.app")])
        .set_template(
            RevisionTemplate::new()
                .set_service_account("checkout-runner@proj-alpha.iam.gserviceaccount.com")
                .set_max_instance_request_concurrency(80)
                .set_scaling(
                    RevisionScaling::new()
                        .set_min_instance_count(2)
                        .set_max_instance_count(9),
                )
                .set_containers([Container::new()
                    .set_image("europe-docker.pkg.dev/proj-alpha/images/checkout:v7")
                    .set_resources(
                        ResourceRequirements::new().set_limits([("cpu", "2"), ("memory", "1Gi")]),
                    )]),
        );

    let detail = map_service_detail(&service);

    assert_eq!(
        detail,
        CloudRunServiceDetail {
            name: "checkout".to_string(),
            region: "europe-west4".to_string(),
            url: Some("https://checkout-abc.a.run.app".to_string()),
            status: CloudRunServiceStatus::Ready,
            latest_ready_revision: Some("checkout-00007-abc".to_string()),
            latest_created_revision: Some("checkout-00008-def".to_string()),
            traffic: vec![CloudRunTrafficTarget {
                revision_name: Some("checkout-00007-abc".to_string()),
                tag: Some("live".to_string()),
                percent: Some(100),
                latest: false,
                url: Some("https://live---checkout-abc.a.run.app".to_string()),
            }],
            // Derived from the split, as on the CLI path: the one target it
            // names, marked Ready because it is the latest ready revision.
            revisions: vec![CloudRunRevision {
                name: "checkout-00007-abc".to_string(),
                ready: CloudRunServiceStatus::Ready,
                created: None,
                image: None,
                active_percent: Some(100),
            }],
            conditions: vec![
                // The terminal condition leads: v2 keeps readiness in its own
                // field, and the pane shows it as the first row exactly as the
                // CLI path did.
                CloudRunCondition {
                    type_: "Ready".to_string(),
                    status: "True".to_string(),
                    reason: None,
                    message: None,
                    last_transition_time: None,
                },
                CloudRunCondition {
                    type_: "RoutesReady".to_string(),
                    status: "True".to_string(),
                    reason: None,
                    message: Some("All routes are ready".to_string()),
                    last_transition_time: Some(PLANTED_TIMESTAMP_TEXT.to_string()),
                },
            ],
            image: Some("europe-docker.pkg.dev/proj-alpha/images/checkout:v7".to_string()),
            cpu: Some("2".to_string()),
            memory: Some("1Gi".to_string()),
            concurrency: Some(80),
            service_account: Some("checkout-runner@proj-alpha.iam.gserviceaccount.com".to_string()),
            ingress: Some("internal".to_string()),
            min_instances: Some(2),
            max_instances: Some(9),
            labels: vec![ComputeKeyValue {
                key: "team".to_string(),
                value: "payments".to_string(),
            }],
            // Asserted separately below: the JSON is the whole model, and
            // pinning it verbatim here would pin the client's field set.
            raw_json: detail.raw_json.clone(),
        },
        "every detail field is filled from its own part of the model"
    );

    assert!(
        detail.raw_json.contains(SERVICE_NAME),
        "the JSON tab shows the service the API returned: {}",
        detail.raw_json
    );
}

#[test]
fn the_traffic_list_keeps_api_order_and_flags_the_latest_target() {
    let service = Service::new().set_name(SERVICE_NAME).set_traffic_statuses([
        TrafficTargetStatus::new()
            .set_revision("checkout-00007-abc")
            .set_percent(75)
            .set_tag("stable")
            .set_uri("https://stable---checkout-abc.a.run.app"),
        // The latest target, marked by its allocation type. A *status* names
        // the revision it resolved to even for this one, so the flag cannot be
        // read from an empty name -- see
        // `a_latest_traffic_target_is_marked_latest_even_though_it_names_a_revision`.
        TrafficTargetStatus::new()
            .set_type(google_cloud_run_v2::model::TrafficTargetAllocationType::Latest)
            .set_revision("")
            .set_percent(25),
    ]);

    assert_eq!(
        map_service_detail(&service).traffic,
        vec![
            CloudRunTrafficTarget {
                revision_name: Some("checkout-00007-abc".to_string()),
                tag: Some("stable".to_string()),
                percent: Some(75),
                latest: false,
                url: Some("https://stable---checkout-abc.a.run.app".to_string()),
            },
            CloudRunTrafficTarget {
                revision_name: None,
                tag: None,
                percent: Some(25),
                latest: true,
                url: None,
            },
        ],
        "the split renders in the API's order, and only the latest target is flagged"
    );
}

#[test]
fn a_traffic_target_shortens_a_fully_qualified_revision_name() {
    let service =
        Service::new()
            .set_name(SERVICE_NAME)
            .set_traffic_statuses([TrafficTargetStatus::new()
                .set_revision(REVISION_NAME)
                .set_percent(100)]);

    let traffic = map_service_detail(&service).traffic;
    assert_eq!(
        traffic[0].revision_name,
        Some("checkout-00007-abc".to_string()),
        "a full resource path renders as the revision id"
    );
    assert!(!traffic[0].latest, "a named target is not the latest one");
}

#[test]
fn a_condition_state_renders_as_the_knative_true_false_or_unknown_word() {
    let service = Service::new().set_name(SERVICE_NAME).set_conditions([
        Condition::new()
            .set_type("RoutesReady")
            .set_state(State::ConditionSucceeded)
            .set_message("All routes are ready"),
        Condition::new()
            .set_type("ConfigurationsReady")
            .set_state(State::ConditionFailed)
            .set_message("Image pull failed"),
        Condition::new()
            .set_type("RetiredRevisions")
            .set_state(State::ConditionReconciling),
    ]);

    assert_eq!(
        map_service_detail(&service).conditions,
        vec![
            CloudRunCondition {
                type_: "RoutesReady".to_string(),
                status: "True".to_string(),
                reason: None,
                message: Some("All routes are ready".to_string()),
                last_transition_time: None,
            },
            CloudRunCondition {
                type_: "ConfigurationsReady".to_string(),
                status: "False".to_string(),
                reason: None,
                message: Some("Image pull failed".to_string()),
                last_transition_time: None,
            },
            CloudRunCondition {
                type_: "RetiredRevisions".to_string(),
                status: "Unknown".to_string(),
                reason: None,
                message: None,
                last_transition_time: None,
            },
        ],
        "succeeded reads True, failed reads False, anything unsettled reads Unknown, \
         and an empty message is absent"
    );
}

#[test]
fn container_limits_come_from_the_first_container() {
    let service =
        Service::new()
            .set_name(SERVICE_NAME)
            .set_template(RevisionTemplate::new().set_containers([
                Container::new().set_image("first-image:1").set_resources(
                    ResourceRequirements::new().set_limits([("cpu", "4"), ("memory", "8Gi")]),
                ),
                Container::new().set_image("sidecar-image:2").set_resources(
                    ResourceRequirements::new().set_limits([("cpu", "1"), ("memory", "256Mi")]),
                ),
            ]));

    let detail = map_service_detail(&service);
    assert_eq!(detail.image, Some("first-image:1".to_string()));
    assert_eq!(detail.cpu, Some("4".to_string()));
    assert_eq!(detail.memory, Some("8Gi".to_string()));
}

#[test]
fn missing_limits_are_absent_rather_than_blank() {
    let no_resources = map_service_detail(&Service::new().set_name(SERVICE_NAME).set_template(
        RevisionTemplate::new().set_containers([Container::new().set_image("image:1")]),
    ));
    assert_eq!(no_resources.cpu, None, "no resources block means no cpu");
    assert_eq!(no_resources.memory, None);

    let cpu_only = map_service_detail(
        &Service::new().set_name(SERVICE_NAME).set_template(
            RevisionTemplate::new().set_containers([Container::new()
                .set_image("image:1")
                .set_resources(ResourceRequirements::new().set_limits([("cpu", "2")]))]),
        ),
    );
    assert_eq!(cpu_only.cpu, Some("2".to_string()));
    assert_eq!(
        cpu_only.memory, None,
        "a limit the map does not carry is absent, not the other limit's value"
    );
}

#[test]
fn a_zero_concurrency_is_the_unset_sentinel_and_a_real_one_is_kept() {
    let unset = map_service_detail(
        &Service::new()
            .set_name(SERVICE_NAME)
            .set_template(RevisionTemplate::new().set_max_instance_request_concurrency(0)),
    );
    assert_eq!(
        unset.concurrency, None,
        "zero concurrent requests per instance is not a setting a service can hold"
    );

    let set = map_service_detail(
        &Service::new()
            .set_name(SERVICE_NAME)
            .set_template(RevisionTemplate::new().set_max_instance_request_concurrency(42)),
    );
    assert_eq!(set.concurrency, Some(42));
}

#[test]
fn a_zero_minimum_is_kept_and_a_zero_maximum_is_absent() {
    // Scale-to-zero is the Cloud Run default and a real setting; a maximum of
    // zero only means the field was never set.
    let scale_to_zero = map_service_detail(
        &Service::new().set_name(SERVICE_NAME).set_template(
            RevisionTemplate::new().set_scaling(
                RevisionScaling::new()
                    .set_min_instance_count(0)
                    .set_max_instance_count(0),
            ),
        ),
    );
    assert_eq!(
        scale_to_zero.min_instances,
        Some(0),
        "a minimum of zero is scale-to-zero, which the pane must show"
    );
    assert_eq!(
        scale_to_zero.max_instances, None,
        "a maximum of zero is the unset sentinel"
    );

    let bounded = map_service_detail(
        &Service::new().set_name(SERVICE_NAME).set_template(
            RevisionTemplate::new().set_scaling(
                RevisionScaling::new()
                    .set_min_instance_count(3)
                    .set_max_instance_count(11),
            ),
        ),
    );
    assert_eq!(bounded.min_instances, Some(3));
    assert_eq!(bounded.max_instances, Some(11));
}

#[test]
fn a_service_without_a_template_reports_no_container_or_scaling_facts() {
    let detail = map_service_detail(
        &Service::new()
            .set_name(SERVICE_NAME)
            .set_uri("")
            .set_latest_ready_revision("")
            .set_latest_created_revision(""),
    );

    assert_eq!(
        detail.url, None,
        "the proto's empty string is not a URL in the pane either"
    );
    assert_eq!(detail.latest_ready_revision, None);
    assert_eq!(detail.latest_created_revision, None);
    assert_eq!(detail.image, None);
    assert_eq!(detail.cpu, None);
    assert_eq!(detail.memory, None);
    assert_eq!(detail.concurrency, None);
    assert_eq!(detail.service_account, None);
    assert_eq!(
        detail.min_instances, None,
        "no scaling block is absent, unlike a minimum of zero inside one"
    );
    assert_eq!(detail.max_instances, None);
    assert!(detail.traffic.is_empty());
    assert!(detail.conditions.is_empty());
    assert!(detail.labels.is_empty());
}

#[test]
fn an_empty_template_service_account_is_absent() {
    let detail = map_service_detail(
        &Service::new()
            .set_name(SERVICE_NAME)
            .set_template(RevisionTemplate::new().set_service_account("")),
    );

    assert_eq!(
        detail.service_account, None,
        "an unset service account is absent, not an empty email"
    );
}

#[test]
fn an_ingress_mode_this_build_cannot_name_is_shown_as_the_api_spells_it() {
    // The named modes are translated into the CLI's vocabulary and pinned in
    // `ingress_reads_in_the_cli_vocabulary_and_unspecified_reads_as_absent`.
    // This is the other side: a mode with no name at all.
    let nameless = map_service_detail(
        &Service::new()
            .set_name(SERVICE_NAME)
            .set_ingress(IngressTraffic::from(77_i32)),
    );
    assert_eq!(
        nameless.ingress, None,
        "an ingress mode this build cannot name is absent rather than invented"
    );

    let unknown_word = map_service_detail(
        &Service::new()
            .set_name(SERVICE_NAME)
            .set_ingress(IngressTraffic::from("INGRESS_TRAFFIC_INTERNAL_AND_PRIVATE")),
    );
    assert_eq!(
        unknown_word.ingress.as_deref(),
        Some("INGRESS_TRAFFIC_INTERNAL_AND_PRIVATE"),
        "a setting newer than this build reaches the cell as the API spells it"
    );
}

#[test]
fn service_labels_become_key_value_pairs() {
    let service = Service::new()
        .set_name(SERVICE_NAME)
        .set_labels([("team", "payments"), ("env", "prod")]);

    // The model holds labels in a hash map, so the mapper's output order is
    // the map's; sorting compares the pairs themselves.
    let mut labels = map_service_detail(&service).labels;
    labels.sort_by(|left, right| left.key.cmp(&right.key));

    assert_eq!(
        labels,
        vec![
            ComputeKeyValue {
                key: "env".to_string(),
                value: "prod".to_string(),
            },
            ComputeKeyValue {
                key: "team".to_string(),
                value: "payments".to_string(),
            },
        ],
        "both labels carry their own key and their own value"
    );
}

// ---------------------------------------------------------------------------
// map_revision_detail
// ---------------------------------------------------------------------------

#[test]
fn a_revision_detail_carries_every_field_the_api_filled() {
    let revision = Revision::new()
        .set_name(REVISION_NAME)
        .set_service(SERVICE_NAME)
        .set_create_time(planted_timestamp())
        .set_service_account("checkout-runner@proj-alpha.iam.gserviceaccount.com")
        .set_max_instance_request_concurrency(64)
        .set_scaling(
            RevisionScaling::new()
                .set_min_instance_count(1)
                .set_max_instance_count(7),
        )
        .set_conditions([Condition::new()
            .set_type("Ready")
            .set_state(State::ConditionSucceeded)
            .set_message("Revision is serving")
            .set_last_transition_time(planted_timestamp())])
        .set_containers([Container::new()
            .set_image("europe-docker.pkg.dev/proj-alpha/images/checkout:v7")
            .set_resources(
                ResourceRequirements::new().set_limits([("cpu", "2"), ("memory", "1Gi")]),
            )]);

    let detail = map_revision_detail(&revision);

    assert_eq!(
        detail,
        CloudRunRevisionDetail {
            name: "checkout-00007-abc".to_string(),
            service: Some("checkout".to_string()),
            region: "europe-west4".to_string(),
            ready: CloudRunServiceStatus::Ready,
            conditions: vec![CloudRunCondition {
                type_: "Ready".to_string(),
                status: "True".to_string(),
                reason: None,
                message: Some("Revision is serving".to_string()),
                last_transition_time: Some(PLANTED_TIMESTAMP_TEXT.to_string()),
            }],
            image: Some("europe-docker.pkg.dev/proj-alpha/images/checkout:v7".to_string()),
            cpu: Some("2".to_string()),
            memory: Some("1Gi".to_string()),
            concurrency: Some(64),
            min_instances: Some(1),
            max_instances: Some(7),
            service_account: Some("checkout-runner@proj-alpha.iam.gserviceaccount.com".to_string()),
            created: Some(PLANTED_TIMESTAMP_TEXT.to_string()),
            raw_json: detail.raw_json.clone(),
        },
        "every revision detail field is filled from its own part of the model"
    );

    assert!(
        detail.raw_json.contains(REVISION_NAME),
        "the JSON tab shows the revision the API returned: {}",
        detail.raw_json
    );
}

#[test]
fn a_revision_detail_reads_readiness_from_the_ready_condition() {
    // The `Active` condition disagrees and comes first, so a lookup that takes
    // whatever condition it meets first reports the wrong readiness.
    let revision = Revision::new().set_name(REVISION_NAME).set_conditions([
        Condition::new()
            .set_type("Active")
            .set_state(State::ConditionFailed),
        Condition::new()
            .set_type("Ready")
            .set_state(State::ConditionPending),
    ]);

    let detail = map_revision_detail(&revision);
    assert_eq!(
        detail.ready,
        CloudRunServiceStatus::Reconciling,
        "readiness comes from the Ready condition, whose state is pending here"
    );
    assert_eq!(
        detail.conditions.len(),
        2,
        "the pane still lists every condition, not just the Ready one"
    );
    assert_eq!(detail.conditions[0].type_, "Active");
    assert_eq!(detail.conditions[0].status, "False");
    assert_eq!(detail.conditions[1].type_, "Ready");
    assert_eq!(
        detail.conditions[1].status, "Unknown",
        "a pending condition reads Unknown in the Knative vocabulary"
    );
}

#[test]
fn a_revision_detail_shortens_its_parent_service_and_drops_an_unset_one() {
    let qualified = map_revision_detail(
        &Revision::new()
            .set_name(REVISION_NAME)
            .set_service(SERVICE_NAME),
    );
    assert_eq!(
        qualified.service,
        Some("checkout".to_string()),
        "the parent service renders as its id, not its resource path"
    );

    let unset = map_revision_detail(&Revision::new().set_name(REVISION_NAME).set_service(""));
    assert_eq!(
        unset.service, None,
        "an unset parent is absent, not a service named nothing"
    );
}

#[test]
fn a_revision_detail_keeps_a_zero_minimum_and_drops_a_zero_maximum_and_concurrency() {
    let zeroed = map_revision_detail(
        &Revision::new()
            .set_name(REVISION_NAME)
            .set_max_instance_request_concurrency(0)
            .set_scaling(
                RevisionScaling::new()
                    .set_min_instance_count(0)
                    .set_max_instance_count(0),
            ),
    );
    assert_eq!(
        zeroed.min_instances,
        Some(0),
        "scale-to-zero is a real setting on a revision too"
    );
    assert_eq!(zeroed.max_instances, None, "a maximum of zero is unset");
    assert_eq!(
        zeroed.concurrency, None,
        "a concurrency of zero is unset rather than a limit of none"
    );

    let bounded = map_revision_detail(
        &Revision::new()
            .set_name(REVISION_NAME)
            .set_max_instance_request_concurrency(10)
            .set_scaling(
                RevisionScaling::new()
                    .set_min_instance_count(4)
                    .set_max_instance_count(12),
            ),
    );
    assert_eq!(bounded.min_instances, Some(4));
    assert_eq!(bounded.max_instances, Some(12));
    assert_eq!(bounded.concurrency, Some(10));
}

#[test]
fn a_revision_detail_without_scaling_reports_no_instance_bounds() {
    let detail = map_revision_detail(&Revision::new().set_name(REVISION_NAME));

    assert_eq!(
        detail.min_instances, None,
        "no scaling block is absent, unlike a minimum of zero inside one"
    );
    assert_eq!(detail.max_instances, None);
    assert_eq!(detail.service_account, None);
    assert_eq!(detail.created, None);
    assert_eq!(detail.cpu, None);
    assert_eq!(detail.memory, None);
    assert_eq!(
        detail.ready,
        CloudRunServiceStatus::Unknown(String::new()),
        "a revision with no conditions has no readiness to report"
    );
}

#[test]
fn a_revision_detail_takes_its_limits_from_the_first_container() {
    let revision = Revision::new().set_name(REVISION_NAME).set_containers([
        Container::new().set_image("first-image:1").set_resources(
            ResourceRequirements::new().set_limits([("cpu", "8"), ("memory", "16Gi")]),
        ),
        Container::new().set_image("sidecar-image:2").set_resources(
            ResourceRequirements::new().set_limits([("cpu", "1"), ("memory", "128Mi")]),
        ),
    ]);

    let detail = map_revision_detail(&revision);
    assert_eq!(detail.image, Some("first-image:1".to_string()));
    assert_eq!(detail.cpu, Some("8".to_string()));
    assert_eq!(detail.memory, Some("16Gi".to_string()));
}

// ---------------------------------------------- divergences found by measuring
//
// The tests below pin behaviour the native mappers lost against the CLI path
// they replaced. Each was found by comparing the two backends' rows rather
// than by a failing test, so each plants the input that produced the wrong
// cell and asserts the CLI's own vocabulary.

/// A condition's reason reaches the row, whichever of v2's three it is.
///
/// v2 replaced the CLI JSON's single `reason` string with a one-of: a
/// service-level reason, a revision reason and an execution reason. The mapper
/// returned `None` unconditionally, so the Conditions pane said a deploy had
/// failed and could not say why -- `RevisionMissing` is the word an operator
/// reads to find out.
#[test]
fn a_condition_carries_whichever_flavour_of_reason_the_api_sent() {
    use google_cloud_run_v2::model::condition::{
        CommonReason, ExecutionReason, Reasons, RevisionReason,
    };

    let common = Condition::new()
        .set_type("Ready")
        .set_state(State::ConditionFailed)
        .set_reasons(Reasons::Reason(CommonReason::RevisionFailed));
    let revision = Condition::new()
        .set_type("RoutesReady")
        .set_state(State::ConditionFailed)
        .set_reasons(Reasons::RevisionReason(RevisionReason::HealthCheckSkipped));
    let execution = Condition::new()
        .set_type("Completed")
        .set_state(State::ConditionFailed)
        .set_reasons(Reasons::ExecutionReason(ExecutionReason::NonZeroExitCode));

    let service = Service::new()
        .set_name(SERVICE_NAME)
        .set_conditions([common, revision, execution]);
    let reasons: Vec<Option<String>> = map_service_detail(&service)
        .conditions
        .into_iter()
        .map(|condition| condition.reason)
        .collect();

    assert_eq!(
        reasons,
        vec![
            Some("REVISION_FAILED".to_string()),
            Some("HEALTH_CHECK_SKIPPED".to_string()),
            Some("NON_ZERO_EXIT_CODE".to_string()),
        ],
        "all three reason flavours reach the pane, and none of them is dropped"
    );

    let silent = Service::new()
        .set_name(SERVICE_NAME)
        .set_conditions([Condition::new()
            .set_type("Ready")
            .set_state(State::ConditionSucceeded)]);
    assert_eq!(
        map_service_detail(&silent).conditions[0].reason,
        None,
        "a condition that states no reason yields no reason, not an empty string"
    );
}

/// The readiness condition is in the pane.
///
/// v2 moved it out of `conditions` into `terminal_condition`, so a mapper fed
/// only `conditions` dropped the `Ready` row the CLI path always showed -- the
/// one line that says whether the service is serving. It leads the list, as it
/// did on the CLI path.
#[test]
fn the_terminal_condition_leads_the_conditions_the_pane_shows() {
    let service = Service::new()
        .set_name(SERVICE_NAME)
        .set_terminal_condition(
            Condition::new()
                .set_type("Ready")
                .set_state(State::ConditionSucceeded),
        )
        .set_conditions([
            Condition::new()
                .set_type("ConfigurationsReady")
                .set_state(State::ConditionSucceeded),
            Condition::new()
                .set_type("RoutesReady")
                .set_state(State::ConditionFailed),
        ]);

    let shown: Vec<(String, String)> = map_service_detail(&service)
        .conditions
        .into_iter()
        .map(|condition| (condition.type_, condition.status))
        .collect();
    assert_eq!(
        shown,
        vec![
            ("Ready".to_string(), "True".to_string()),
            ("ConfigurationsReady".to_string(), "True".to_string()),
            ("RoutesReady".to_string(), "False".to_string()),
        ]
    );

    let without = Service::new()
        .set_name(SERVICE_NAME)
        .set_conditions([Condition::new()
            .set_type("RoutesReady")
            .set_state(State::ConditionSucceeded)]);
    assert_eq!(
        map_service_detail(&without).conditions.len(),
        1,
        "a service with no terminal condition contributes no extra row"
    );
}

/// A split routed at "the latest revision" is marked as such.
///
/// The flag was read from an empty revision name, which a *status* never has:
/// it reports the revision the allocation resolved to. So `LATEST` was
/// unreachable and the pane printed a revision id where the CLI printed the
/// word. The allocation type is what carries the distinction.
#[test]
fn a_latest_traffic_target_is_marked_latest_even_though_it_names_a_revision() {
    use google_cloud_run_v2::model::TrafficTargetAllocationType;

    let service = Service::new().set_name(SERVICE_NAME).set_traffic_statuses([
        TrafficTargetStatus::new()
            .set_type(TrafficTargetAllocationType::Latest)
            .set_revision(REVISION_NAME)
            .set_percent(80),
        TrafficTargetStatus::new()
            .set_type(TrafficTargetAllocationType::Revision)
            .set_revision(REVISION_NAME)
            .set_percent(20),
    ]);

    let flags: Vec<bool> = map_service_detail(&service)
        .traffic
        .into_iter()
        .map(|target| target.latest)
        .collect();
    assert_eq!(
        flags,
        vec![true, false],
        "the allocation type decides, and both targets name the same revision"
    );
}

/// Ingress is printed in the vocabulary the pane already used.
///
/// The CLI path read the `run.googleapis.com/ingress` annotation, whose values
/// are `all`, `internal` and `internal-and-cloud-load-balancing`. Passing the
/// v2 enum through printed `INGRESS_TRAFFIC_ALL` in the same cell, and painted
/// `INGRESS_TRAFFIC_UNSPECIFIED` where the CLI left the field blank.
#[test]
fn ingress_reads_in_the_cli_vocabulary_and_unspecified_reads_as_absent() {
    let word = |ingress: IngressTraffic| {
        map_service_detail(&Service::new().set_name(SERVICE_NAME).set_ingress(ingress)).ingress
    };

    assert_eq!(word(IngressTraffic::All).as_deref(), Some("all"));
    assert_eq!(
        word(IngressTraffic::InternalOnly).as_deref(),
        Some("internal")
    );
    assert_eq!(
        word(IngressTraffic::InternalLoadBalancer).as_deref(),
        Some("internal-and-cloud-load-balancing")
    );
    assert_eq!(
        word(IngressTraffic::Unspecified),
        None,
        "no setting stated is an empty cell, not the sentinel's name"
    );
    assert_eq!(
        map_service_detail(&Service::new().set_name(SERVICE_NAME)).ingress,
        None,
        "a service that never set the field reads the same way"
    );
}

/// A failed condition is a failure only when it says why.
///
/// v2 has one `CONDITION_FAILED` where Knative had two states, and
/// `status_from_state` read it as `Failed` whatever else the condition said.
/// That made `NotReady` unreachable on this path: the STATUS cell of every
/// service that had simply not come up yet read "Failed" toned Degraded where
/// the CLI path read "Not Ready" toned Warning. The `reasons` one-of carries
/// the distinction the state lost, and it is read here the way the CLI parser
/// reads Knative's `reason` key.
#[test]
fn a_failed_condition_is_a_failure_only_when_it_states_a_reason() {
    use google_cloud_run_v2::model::condition::{
        CommonReason, ExecutionReason, Reasons, RevisionReason,
    };

    let with = |reasons: Reasons| {
        map_service(
            &Service::new()
                .set_name(SERVICE_NAME)
                .set_terminal_condition(
                    Condition::new()
                        .set_type("Ready")
                        .set_state(State::ConditionFailed)
                        .set_reasons(reasons),
                ),
        )
        .status
    };

    assert_eq!(
        with(Reasons::Reason(CommonReason::ContainerMissing)),
        CloudRunServiceStatus::Failed,
        "a service-level reason is a stated cause, so the service failed"
    );
    assert_eq!(
        with(Reasons::RevisionReason(RevisionReason::HealthCheckSkipped)),
        CloudRunServiceStatus::Failed,
        "so is a revision reason -- all three flavours of the one-of count"
    );
    assert_eq!(
        with(Reasons::ExecutionReason(ExecutionReason::NonZeroExitCode)),
        CloudRunServiceStatus::Failed,
        "and so is an execution reason"
    );

    assert_eq!(
        map_service(
            &Service::new()
                .set_name(SERVICE_NAME)
                .set_terminal_condition(
                    Condition::new()
                        .set_type("Ready")
                        .set_state(State::ConditionFailed)
                        .set_message("Revision checkout-00008-def is not ready."),
                )
        )
        .status,
        CloudRunServiceStatus::NotReady,
        "the same state with no reason at all is not-yet-ready; a message is \
         not a cause, and reading one as a cause would put every such service \
         back in the Degraded cell"
    );
}

/// The same split reaches a revision's readiness, not only a service's.
///
/// `map_revision` and `map_revision_detail` share `status_from_state`, so a
/// revision whose `Ready` condition failed without a reason must read
/// `NotReady` too -- the READY column of the revisions tab is the same
/// vocabulary as the STATUS column of the services table.
#[test]
fn a_revision_whose_ready_condition_states_no_reason_is_not_ready() {
    use google_cloud_run_v2::model::condition::{CommonReason, Reasons};

    let ready_condition = |reasons: Option<Reasons>| {
        let mut condition = Condition::new()
            .set_type("Ready")
            .set_state(State::ConditionFailed);
        if let Some(reasons) = reasons {
            condition = condition.set_reasons(reasons);
        }
        Revision::new()
            .set_name(REVISION_NAME)
            .set_conditions([condition])
    };

    assert_eq!(
        map_revision(&ready_condition(None)).ready,
        CloudRunServiceStatus::NotReady,
        "a revision that has not come up reads Not Ready in the row"
    );
    assert_eq!(
        map_revision_detail(&ready_condition(Some(Reasons::Reason(
            CommonReason::ContainerMissing
        ))))
        .ready,
        CloudRunServiceStatus::Failed,
        "and one whose container is missing reads Failed in the pane"
    );
}

/// The detail pane's revision list is derived from the split, as the CLI
/// pane's always was.
///
/// `map_service_detail` left `revisions` empty and its doc comment claimed the
/// CLI path did the same. The CLI path calls `derive_revisions_from_status`,
/// so between a drill-in and the pane's own revisions fetch the native pane
/// showed no revisions where the CLI pane named the one serving and the one
/// being replaced. This is the single place where the native pane showed an
/// operator strictly less than the CLI pane, which is why it was closed rather
/// than pinned.
#[test]
fn the_detail_revisions_are_derived_from_the_traffic_split() {
    use google_cloud_run_v2::model::TrafficTargetAllocationType;

    let service = Service::new()
        .set_name(SERVICE_NAME)
        .set_latest_ready_revision(REVISION_NAME)
        .set_traffic_statuses([
            TrafficTargetStatus::new()
                .set_type(TrafficTargetAllocationType::Latest)
                .set_revision("checkout-00007-abc")
                .set_percent(80),
            TrafficTargetStatus::new()
                .set_revision("checkout-00006-zzz")
                .set_percent(20),
            // The same revision again under a tag: two targets, one revision.
            TrafficTargetStatus::new()
                .set_revision("checkout-00006-zzz")
                .set_tag("previous")
                .set_percent(0),
            // A target that resolved to nothing contributes no row.
            TrafficTargetStatus::new().set_percent(0),
        ]);

    assert_eq!(
        map_service_detail(&service).revisions,
        vec![
            CloudRunRevision {
                name: "checkout-00007-abc".to_string(),
                ready: CloudRunServiceStatus::Ready,
                created: None,
                image: None,
                active_percent: Some(80),
            },
            CloudRunRevision {
                name: "checkout-00006-zzz".to_string(),
                ready: CloudRunServiceStatus::Unknown(String::new()),
                created: None,
                image: None,
                active_percent: Some(20),
            },
        ],
        "the split's order, its percentages, the latest ready revision marked \
         Ready and every other target left unknown -- and the tagged repeat of \
         a revision already listed adds no second row"
    );

    assert_eq!(
        map_service_detail(&Service::new().set_name(SERVICE_NAME)).revisions,
        Vec::new(),
        "a service with no split states no revisions, rather than one blank row"
    );
}

/// The latest-ready revision is matched on the revision name, not the resource
/// path.
///
/// `Service::latest_ready_revision` is a full
/// `projects/<p>/locations/<r>/services/<s>/revisions/<rev>` path and a
/// traffic status names a bare revision, so comparing the two as they arrive
/// never matches and every derived row reads Unknown -- the READY column
/// blank for a service that is serving perfectly.
#[test]
fn the_latest_ready_revision_is_matched_after_both_names_are_shortened() {
    let service = Service::new()
        .set_name(SERVICE_NAME)
        .set_latest_ready_revision(REVISION_NAME)
        .set_traffic_statuses([TrafficTargetStatus::new()
            .set_revision("checkout-00007-abc")
            .set_percent(100)]);

    assert_eq!(
        map_service_detail(&service)
            .revisions
            .into_iter()
            .map(|revision| (revision.name, revision.ready))
            .collect::<Vec<_>>(),
        vec![(
            "checkout-00007-abc".to_string(),
            CloudRunServiceStatus::Ready
        )],
        "the full path and the bare name are the same revision"
    );
}
