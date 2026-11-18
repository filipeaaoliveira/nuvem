//! `map_function`, the one translation between the native Cloud Functions v2
//! model and a `CloudFunction` row — reported untested by the mutation sweep.
//!
//! Every field gets a distinct, non-default value so that a body replaced by
//! `Default::default()` cannot pass, and every branch the mapper reaches
//! through — the two `from_api` enum ladders including their unknown arms,
//! `non_empty` on the URL, `timestamp_to_string` on the update time, the
//! `locations/<region>` scan of the resource path, and the sorted label map —
//! is asserted on both sides with outcomes that differ.

use google_cloud_functions_v2::model::Environment;
use google_cloud_functions_v2::model::Function;
use google_cloud_functions_v2::model::function::State;
use google_cloud_wkt::Timestamp;

use nuvem::services::sdk::functions::map_function;
use nuvem_domain::gcp::resource::{
    CloudFunction, CloudFunctionEnvironment, CloudFunctionState, ComputeKeyValue,
};

/// A deployed function's resource path. The project id and the region are
/// deliberately different words, so a mapper that returned the wrong path
/// segment cannot look right.
const PATH: &str = "projects/acme-prod/locations/europe-west4/functions/thumbnailer";

fn label(key: &str, value: &str) -> ComputeKeyValue {
    ComputeKeyValue {
        key: key.to_string(),
        value: value.to_string(),
    }
}

/// A function with every mapped field set to its own non-default value.
fn populated() -> Function {
    Function::new()
        .set_name(PATH)
        .set_state(State::Deploying)
        .set_environment(Environment::Gen1)
        .set_url("https://europe-west4-acme-prod.cloudfunctions.net/thumbnailer")
        .set_update_time(Timestamp::new(1_750_000_000, 123_456_789).expect("a valid timestamp"))
        .set_labels([("team", "media"), ("env", "prod")])
}

#[test]
fn every_mapped_field_is_read_from_its_own_source() {
    // Fields the row does not carry are populated too: a mapper that read one
    // of them into a row field would have to differ from this expectation.
    let function = populated()
        .set_description("resizes uploaded images")
        .set_kms_key_name("projects/acme-prod/locations/europe-west4/keyRings/r/cryptoKeys/k")
        .set_satisfies_pzs(true);

    assert_eq!(
        map_function(&function),
        CloudFunction {
            name: "thumbnailer".to_string(),
            region: "europe-west4".to_string(),
            state: CloudFunctionState::Deploying,
            environment: CloudFunctionEnvironment::Gen1,
            url: Some("https://europe-west4-acme-prod.cloudfunctions.net/thumbnailer".to_string()),
            update_time: Some("2025-06-15T15:06:40Z".to_string()),
            labels: vec![label("env", "prod"), label("team", "media")],
        },
        "every field carries its own distinct value out of the model"
    );
}

#[test]
fn the_name_is_the_last_path_segment_and_not_the_whole_path() {
    let mapped = map_function(&populated());
    assert_eq!(mapped.name, "thumbnailer");
    assert_ne!(mapped.name, PATH, "the resource path is not the row's name");
    assert_ne!(
        mapped.name, "acme-prod",
        "the name is the function id, not the project"
    );
}

#[test]
fn a_bare_function_id_is_its_own_name() {
    let function = Function::new().set_name("thumbnailer");
    assert_eq!(
        map_function(&function).name,
        "thumbnailer",
        "a path with no separator is already the short id"
    );
}

#[test]
fn the_region_is_the_segment_after_locations_not_the_one_after_projects() {
    let mapped = map_function(&populated());
    assert_eq!(mapped.region, "europe-west4");
    assert_ne!(
        mapped.region, "acme-prod",
        "the scan stops at `locations`, not at the first segment it looks at"
    );
    assert_ne!(
        mapped.region, "functions",
        "the region is the value of the `locations` pair, not the next key"
    );
}

#[test]
fn a_path_with_no_locations_pair_yields_an_empty_region() {
    let function = Function::new()
        .set_name("projects/acme-prod/functions/thumbnailer")
        .set_state(State::Active);
    let mapped = map_function(&function);
    assert_eq!(
        mapped.region, "",
        "no location segment means no region, not a guess"
    );
    assert_eq!(
        mapped.name, "thumbnailer",
        "the rest of the row still maps while the region is missing"
    );
}

#[test]
fn a_path_that_ends_at_locations_yields_an_empty_region() {
    let function = Function::new().set_name("projects/acme-prod/locations");
    assert_eq!(
        map_function(&function).region,
        "",
        "a trailing `locations` with nothing after it is an empty region"
    );
}

#[test]
fn every_documented_state_string_maps_to_its_own_variant() {
    let cases = [
        ("STATE_UNSPECIFIED", CloudFunctionState::Unspecified),
        ("ACTIVE", CloudFunctionState::Active),
        ("FAILED", CloudFunctionState::Failed),
        ("DEPLOYING", CloudFunctionState::Deploying),
        ("DELETING", CloudFunctionState::Deleting),
        ("UNKNOWN", CloudFunctionState::Unknown),
        // Not yet variants of the generated enum; they arrive as the API's own
        // string and must still land on their documented state.
        ("DETACHING", CloudFunctionState::Detaching),
        ("DETACH_FAILED", CloudFunctionState::DetachFailed),
    ];
    for (api, expected) in cases {
        let function = Function::new().set_name(PATH).set_state(State::from(api));
        assert_eq!(
            map_function(&function).state,
            expected,
            "`{api}` is its own state"
        );
    }
}

#[test]
fn an_unset_state_is_unspecified_rather_than_a_serving_state() {
    let function = Function::new().set_name(PATH);
    assert_eq!(
        map_function(&function).state,
        CloudFunctionState::Unspecified,
        "the proto zero value is `STATE_UNSPECIFIED`"
    );
}

#[test]
fn an_unrecognised_state_string_is_carried_through_verbatim() {
    let function = Function::new()
        .set_name(PATH)
        .set_state(State::from("QUARANTINED"));
    assert_eq!(
        map_function(&function).state,
        CloudFunctionState::Unrecognized("QUARANTINED".to_string()),
        "a state nuvem does not know keeps the API's own word"
    );
}

#[test]
fn a_state_the_client_only_knows_as_a_number_is_unspecified() {
    // An integer-valued unknown has no name at all, which is the `None` arm of
    // `from_api` — a different fact from an unrecognised string.
    let function = Function::new().set_name(PATH).set_state(State::from(9_999));
    assert_eq!(
        map_function(&function).state,
        CloudFunctionState::Unspecified,
        "a nameless state is unspecified, not an `Unrecognized` empty string"
    );
}

#[test]
fn every_documented_environment_string_maps_to_its_own_generation() {
    let cases = [
        (
            "ENVIRONMENT_UNSPECIFIED",
            CloudFunctionEnvironment::Unspecified,
        ),
        ("GEN_1", CloudFunctionEnvironment::Gen1),
        ("GEN_2", CloudFunctionEnvironment::Gen2),
    ];
    for (api, expected) in cases {
        let function = Function::new()
            .set_name(PATH)
            .set_environment(Environment::from(api));
        assert_eq!(
            map_function(&function).environment,
            expected,
            "`{api}` is its own generation"
        );
    }
}

#[test]
fn an_unset_environment_is_unspecified_rather_than_a_generation() {
    let function = Function::new().set_name(PATH);
    assert_eq!(
        map_function(&function).environment,
        CloudFunctionEnvironment::Unspecified,
        "an absent `environment` is the proto zero value, not 1st gen"
    );
}

#[test]
fn an_unrecognised_environment_string_is_carried_through_verbatim() {
    let function = Function::new()
        .set_name(PATH)
        .set_environment(Environment::from("GEN_3"));
    assert_eq!(
        map_function(&function).environment,
        CloudFunctionEnvironment::Unrecognized("GEN_3".to_string()),
        "a generation nuvem does not know keeps the API's own word"
    );
}

#[test]
fn an_environment_the_client_only_knows_as_a_number_is_unspecified() {
    let function = Function::new()
        .set_name(PATH)
        .set_environment(Environment::from(7));
    assert_eq!(
        map_function(&function).environment,
        CloudFunctionEnvironment::Unspecified,
        "a nameless environment is unspecified"
    );
}

#[test]
fn a_deployed_function_keeps_its_url_and_a_deploying_one_has_none() {
    let deployed = Function::new()
        .set_name(PATH)
        .set_state(State::Active)
        .set_url("https://europe-west4-acme-prod.cloudfunctions.net/thumbnailer");
    assert_eq!(
        map_function(&deployed).url.as_deref(),
        Some("https://europe-west4-acme-prod.cloudfunctions.net/thumbnailer"),
        "a URL the API returned is the URL the row shows"
    );

    let deploying = Function::new().set_name(PATH).set_state(State::Deploying);
    assert_eq!(
        map_function(&deploying).url,
        None,
        "the empty string is how the model spells no URL, and it maps to absent"
    );
}

#[test]
fn an_explicitly_empty_url_string_is_absent_rather_than_an_empty_cell() {
    // The proto3 zero value arriving on the wire, not merely an unset builder:
    // the same `""` the model would carry back from a function with no trigger.
    let function = Function::new()
        .set_name(PATH)
        .set_state(State::Active)
        .set_url("");
    assert_eq!(
        map_function(&function).url,
        None,
        "`\"\"` is how the model spells no URL, and no URL is `None`"
    );
}

#[test]
fn the_url_is_never_the_source_of_the_region() {
    // A URL naming another region must not move the row: the region is read
    // out of the resource path, which is what a function with no URL still has.
    let function = Function::new()
        .set_name(PATH)
        .set_url("https://us-central1-acme-prod.cloudfunctions.net/thumbnailer");
    assert_eq!(
        map_function(&function).region,
        "europe-west4",
        "the path wins over anything the URL happens to spell"
    );
}

#[test]
fn the_update_time_renders_in_the_cli_shape_and_an_absent_one_stays_absent() {
    let stamped = Function::new()
        .set_name(PATH)
        .set_update_time(Timestamp::new(1_700_000_123, 999_000_000).expect("a valid timestamp"));
    assert_eq!(
        map_function(&stamped).update_time.as_deref(),
        Some("2023-11-14T22:15:23Z"),
        "seconds render in UTC and sub-second precision is dropped"
    );

    let unstamped = Function::new().set_name(PATH);
    assert_eq!(
        map_function(&unstamped).update_time,
        None,
        "a function with no update time gets no rendered one"
    );
}

#[test]
fn an_epoch_update_time_renders_rather_than_reading_as_absent() {
    // `Timestamp::default()` *is* the epoch, so a function stamped at second
    // zero and a function with no stamp at all are different facts. Only the
    // second is `None`.
    let epoch = Function::new()
        .set_name(PATH)
        .set_update_time(Timestamp::new(0, 0).expect("the epoch is a valid timestamp"));
    assert_eq!(
        map_function(&epoch).update_time.as_deref(),
        Some("1970-01-01T00:00:00Z"),
        "a zero-valued stamp still renders; absence is what maps to `None`"
    );
}

#[test]
fn labels_are_sorted_by_key_and_keep_each_key_with_its_own_value() {
    let function = Function::new().set_name(PATH).set_labels([
        ("team", "media"),
        ("env", "prod"),
        ("cost-centre", "4412"),
    ]);
    assert_eq!(
        map_function(&function).labels,
        vec![
            label("cost-centre", "4412"),
            label("env", "prod"),
            label("team", "media"),
        ],
        "labels come back in key order, each still carrying its own value"
    );
}

#[test]
fn an_unlabelled_function_maps_to_no_labels() {
    let function = Function::new().set_name(PATH).set_state(State::Active);
    assert_eq!(
        map_function(&function).labels,
        Vec::<ComputeKeyValue>::new(),
        "no labels is an empty list, not a placeholder pair"
    );
}

#[test]
fn a_single_label_survives_the_map_to_pairs() {
    let function = Function::new().set_name(PATH).set_labels([("env", "prod")]);
    assert_eq!(
        map_function(&function).labels,
        vec![label("env", "prod")],
        "a one-entry map is one pair, key and value the right way round"
    );
}

#[test]
fn two_functions_in_one_listing_keep_their_own_identities() {
    let first = populated();
    let second = Function::new()
        .set_name("projects/acme-prod/locations/us-central1/functions/invoicer")
        .set_state(State::Failed)
        .set_environment(Environment::Gen2)
        .set_labels([("env", "staging")]);

    let first = map_function(&first);
    let second = map_function(&second);

    assert_eq!(
        second,
        CloudFunction {
            name: "invoicer".to_string(),
            region: "us-central1".to_string(),
            state: CloudFunctionState::Failed,
            environment: CloudFunctionEnvironment::Gen2,
            url: None,
            update_time: None,
            labels: vec![label("env", "staging")],
        },
        "a second function maps wholly out of its own model, not the first's"
    );

    assert_ne!(first.name, second.name);
    assert_ne!(first.region, second.region);
    assert_ne!(first.state, second.state);
    assert_ne!(first.environment, second.environment);
    assert_ne!(first.labels, second.labels);
    assert_eq!(second.url, None, "the failed function never got a URL");
    assert_eq!(second.update_time, None);
}
