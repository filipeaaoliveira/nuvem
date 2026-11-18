//! Pins the resource enums' table renderings and API spellings.
//!
//! The `api_value` sides are pinned as `from_api` round-trips: a stubbed
//! `api_value` cannot round-trip, and the pairing keeps the two spellings of
//! each state from drifting apart. The `label` sides are explicit tables —
//! labels are what the operator reads, so each one is spelled out.

use nuvem_domain::gcp::resource::{
    BigQueryTableKind, CloudFunctionEnvironment, CloudFunctionState, CloudSqlAvailability,
    DiskStatus, FirewallAction, FirewallDirection, PubSubDelivery, PubSubSubscriptionState,
    ResourceServiceKind, SecretVersionState, SubscriptionCount, VpcRoutingMode, VpcSubnetMode,
};

#[test]
fn every_service_kind_label_is_its_own() {
    for (kind, expected) in [
        (ResourceServiceKind::ComputeInstances, "GCE instances"),
        (ResourceServiceKind::GkeClusters, "GKE clusters"),
        (ResourceServiceKind::CloudSql, "Cloud SQL"),
        (ResourceServiceKind::CloudRunServices, "Cloud Run"),
        (ResourceServiceKind::CloudStorage, "Cloud Storage"),
        (ResourceServiceKind::SecretManagerSecrets, "Secret Manager"),
        (ResourceServiceKind::CloudFunctions, "Cloud Functions"),
        (ResourceServiceKind::PersistentDisks, "Persistent disks"),
        (ResourceServiceKind::PubSubTopics, "Pub/Sub topics"),
        (
            ResourceServiceKind::PubSubSubscriptions,
            "Pub/Sub subscriptions",
        ),
        (ResourceServiceKind::BigQueryDatasets, "BigQuery datasets"),
        (ResourceServiceKind::IamServiceAccounts, "Service accounts"),
        (ResourceServiceKind::IamPolicy, "IAM policy"),
        (ResourceServiceKind::VpcNetworks, "VPC networks"),
        (ResourceServiceKind::VpcSubnets, "VPC subnets"),
        (ResourceServiceKind::FirewallRules, "Firewall rules"),
    ] {
        assert_eq!(kind.label(), expected);
    }
    assert_eq!(
        ResourceServiceKind::ALL.len(),
        16,
        "table above is complete"
    );
}

#[test]
fn cloud_sql_availability_labels_and_ha() {
    assert_eq!(CloudSqlAvailability::Zonal.label(), "Zonal");
    assert_eq!(CloudSqlAvailability::Regional.label(), "HA (regional)");
    assert_eq!(CloudSqlAvailability::Unknown.label(), "unknown");
    assert!(CloudSqlAvailability::Regional.is_ha());
    assert!(!CloudSqlAvailability::Zonal.is_ha());
    assert!(!CloudSqlAvailability::Unknown.is_ha());
}

#[test]
fn cloud_function_state_api_values_round_trip_and_labels_render() {
    for state in CloudFunctionState::ALL {
        assert_eq!(
            CloudFunctionState::from_api(Some(state.api_value())),
            state,
            "api_value must round-trip"
        );
    }
    for (state, expected) in [
        (CloudFunctionState::Unspecified, "Unspecified"),
        (CloudFunctionState::Active, "Active"),
        (CloudFunctionState::Failed, "Failed"),
        (CloudFunctionState::Deploying, "Deploying"),
        (CloudFunctionState::Deleting, "Deleting"),
        (CloudFunctionState::Unknown, "Unknown"),
        (CloudFunctionState::Detaching, "Detaching"),
        (CloudFunctionState::DetachFailed, "Detach failed"),
    ] {
        assert_eq!(state.label(), expected);
    }
    let raw = CloudFunctionState::Unrecognized("ODD_STATE".into());
    assert_eq!(raw.api_value(), "ODD_STATE");
    assert_eq!(
        raw.label(),
        "Odd state",
        "an unrecognised state renders as a readable label, not as the constant"
    );
}

#[test]
fn cloud_function_environment_api_values_round_trip_and_labels_render() {
    for environment in CloudFunctionEnvironment::ALL {
        assert_eq!(
            CloudFunctionEnvironment::from_api(Some(environment.api_value())),
            environment,
            "api_value must round-trip"
        );
    }
    assert_eq!(CloudFunctionEnvironment::Unspecified.label(), "unspecified");
    assert_eq!(CloudFunctionEnvironment::Gen1.label(), "1st gen");
    assert_eq!(CloudFunctionEnvironment::Gen2.label(), "2nd gen");
}

#[test]
fn disk_status_api_values_round_trip_and_labels_render() {
    for status in DiskStatus::ALL {
        assert_eq!(
            DiskStatus::from_api(Some(status.api_value())),
            status,
            "api_value must round-trip"
        );
    }
    for (status, expected) in [
        (DiskStatus::Creating, "Creating"),
        (DiskStatus::Restoring, "Restoring"),
        (DiskStatus::Failed, "Failed"),
        (DiskStatus::Ready, "Ready"),
        (DiskStatus::Deleting, "Deleting"),
        (DiskStatus::Unavailable, "Unavailable"),
    ] {
        assert_eq!(status.label(), expected);
    }
}

#[test]
fn pubsub_subscription_state_api_values_round_trip_and_labels_render() {
    for state in PubSubSubscriptionState::ALL {
        assert_eq!(
            PubSubSubscriptionState::from_api(Some(state.api_value())),
            state,
            "api_value must round-trip"
        );
    }
    assert_eq!(PubSubSubscriptionState::Unspecified.label(), "Unspecified");
    assert_eq!(PubSubSubscriptionState::Active.label(), "Active");
    assert_eq!(
        PubSubSubscriptionState::ResourceError.label(),
        "Resource error"
    );
}

#[test]
fn secret_version_state_labels_render() {
    assert_eq!(SecretVersionState::Enabled.label(), "Enabled");
    assert_eq!(SecretVersionState::Disabled.label(), "Disabled");
    assert_eq!(SecretVersionState::Destroyed.label(), "Destroyed");
    assert_eq!(SecretVersionState::Unknown("RAW".into()).label(), "Raw");
}

#[test]
fn subscription_count_renders_a_number_or_a_dash_that_is_not_zero() {
    assert_eq!(SubscriptionCount::Known(3).label(), "3");
    assert_eq!(SubscriptionCount::Known(0).label(), "0");
    assert_eq!(SubscriptionCount::Unavailable.label(), "—");
    assert_eq!(SubscriptionCount::Known(3).known(), Some(3));
    assert_eq!(SubscriptionCount::Known(0).known(), Some(0));
    assert_eq!(SubscriptionCount::Unavailable.known(), None);
}

#[test]
fn pubsub_delivery_labels_and_endpoint_travel_together() {
    let push = PubSubDelivery::Push {
        endpoint: "https://handler.example/push".into(),
    };
    assert_eq!(PubSubDelivery::Pull.label(), "Pull");
    assert_eq!(push.label(), "Push");
    assert_eq!(PubSubDelivery::Pull.endpoint(), None);
    assert_eq!(push.endpoint(), Some("https://handler.example/push"));
}

/// The `TYPE` cell of the BigQuery tables table, spelled out.
///
/// Every string is asserted rather than merely checked non-empty: a `label`
/// nothing pins can return `""` or any other word with the suite still green,
/// which is what `BigQuerySortField::label` did for a whole slice.
///
/// `Other(raw)` is the variant that carries the design decision. BigQuery's
/// `type` is an open set, so an unrecognised value is carried through verbatim
/// rather than folded into `Table` — the reader sees the word the API sent,
/// including its underscores and its case, because inventing a prettier
/// spelling would be a claim about a kind this binary does not know.
#[test]
fn bigquery_table_kind_labels_render_and_an_unknown_kind_is_verbatim() {
    assert_eq!(BigQueryTableKind::Table.label(), "Table");
    assert_eq!(BigQueryTableKind::View.label(), "View");
    assert_eq!(
        BigQueryTableKind::MaterializedView.label(),
        "Materialized view"
    );
    assert_eq!(BigQueryTableKind::External.label(), "External");
    assert_eq!(
        BigQueryTableKind::Other("SNAPSHOT".to_string()).label(),
        "SNAPSHOT",
        "an unrecognised type must reach the cell as BigQuery spelled it"
    );

    // Distinct, so a TYPE cell cannot claim a view and a materialized view are
    // the same thing.
    let labels = [
        BigQueryTableKind::Table.label().to_string(),
        BigQueryTableKind::View.label().to_string(),
        BigQueryTableKind::MaterializedView.label().to_string(),
        BigQueryTableKind::External.label().to_string(),
    ];
    for (index, label) in labels.iter().enumerate() {
        assert!(!label.is_empty(), "{index} has no label");
        assert!(
            !labels[..index].contains(label),
            "{label:?} is used by two kinds"
        );
    }
}

/// A VPC network's `MODE` cell, spelled as gcloud spells it.
///
/// Upper case on purpose: this is a mode name an operator matches against
/// `gcloud compute networks list` output, not prose, so a lower-case
/// rendering would be a different claim about what the two agree on.
///
/// These live in this crate rather than beside the table that paints them
/// so that the domain crate's own test binary pins them: a label assertion in
/// the binary crate only reaches them through the rendering.
#[test]
fn vpc_subnet_mode_labels_are_gclouds_own_spellings() {
    assert_eq!(VpcSubnetMode::Auto.label(), "AUTO");
    assert_eq!(VpcSubnetMode::Custom.label(), "CUSTOM");
    assert_eq!(VpcSubnetMode::Legacy.label(), "LEGACY");
}

/// `autoCreateSubnetworks` decides the mode, and an **absent** field is
/// `LEGACY` rather than the `false` a defaulted `bool` would have produced.
///
/// That default is the defect this constructor exists to prevent: it would
/// paint a legacy network `CUSTOM`, sending an operator to look for subnets
/// that cannot exist.
#[test]
fn an_absent_auto_create_subnetworks_is_legacy_and_not_custom() {
    assert_eq!(VpcSubnetMode::from_api(Some(true)), VpcSubnetMode::Auto);
    assert_eq!(VpcSubnetMode::from_api(Some(false)), VpcSubnetMode::Custom);
    assert_eq!(VpcSubnetMode::from_api(None), VpcSubnetMode::Legacy);
}

/// A network's `ROUTING` cell.
///
/// An unspecified mode paints as the empty string so the column renders its
/// dash rather than a word the API never sent, and an unrecognised one is
/// carried verbatim so a mode added to the API after this build reaches the
/// screen as the API spelled it.
#[test]
fn vpc_routing_mode_labels_render_and_carry_the_unrecognised_verbatim() {
    assert_eq!(VpcRoutingMode::Regional.label(), "REGIONAL");
    assert_eq!(VpcRoutingMode::Global.label(), "GLOBAL");
    assert_eq!(
        VpcRoutingMode::Unspecified.label(),
        "",
        "an unreported routing mode contributes no word, so the column dashes"
    );
    assert_eq!(
        VpcRoutingMode::Unrecognized("INTERPLANETARY".to_string()).label(),
        "INTERPLANETARY"
    );
}

/// An absent or empty `routingConfig.routingMode` is `Unspecified`, not the
/// API's `REGIONAL` default.
///
/// Regional is what the API assigns a network created without one, and that
/// is a claim about creation; this cell is a claim about what the listing
/// reported.
#[test]
fn an_absent_routing_mode_is_unspecified_and_not_the_apis_default() {
    assert_eq!(VpcRoutingMode::from_api(None), VpcRoutingMode::Unspecified);
    assert_eq!(
        VpcRoutingMode::from_api(Some("")),
        VpcRoutingMode::Unspecified
    );
    assert_eq!(
        VpcRoutingMode::from_api(Some("REGIONAL")),
        VpcRoutingMode::Regional
    );
    assert_eq!(
        VpcRoutingMode::from_api(Some("GLOBAL")),
        VpcRoutingMode::Global
    );
    assert_eq!(
        VpcRoutingMode::from_api(Some("INTERPLANETARY")),
        VpcRoutingMode::Unrecognized("INTERPLANETARY".to_string())
    );
}

/// A firewall rule's verdict word, and the shape it is derived from.
///
/// The API does not report an action field. It reports either an `allowed`
/// array or a `denied` array, and the verdict is which of the two is present —
/// so the derivation is a two-bit function, and all four inputs are pinned
/// here including the two the API is documented never to send. A rule with
/// neither array and a rule with both are both `Unspecified` rather than a
/// guess: guessing means painting `ALLOW` over a rule that blocks, or `DENY`
/// over one that does not, and either misreading is worse than a blank cell an
/// operator has to go and check.
#[test]
fn a_firewall_rules_action_is_which_array_the_api_sent() {
    assert_eq!(FirewallAction::from_api(true, false), FirewallAction::Allow);
    assert_eq!(FirewallAction::from_api(false, true), FirewallAction::Deny);
    assert_eq!(
        FirewallAction::from_api(false, false),
        FirewallAction::Unspecified,
        "neither array present: nothing was reported, so nothing is claimed"
    );
    assert_eq!(
        FirewallAction::from_api(true, true),
        FirewallAction::Unspecified,
        "both arrays present is a shape the API is documented never to send; \
         picking one of them would be inventing a verdict"
    );
}

/// The action labels, which are the words the operator reads.
///
/// Spelled out rather than derived: `ALLOW` and `DENY` are gcloud's own
/// spellings, and the empty label is what makes the `Unspecified` case paint a
/// dash instead of a word.
#[test]
fn firewall_action_labels_are_gclouds_own_spellings() {
    assert_eq!(FirewallAction::Allow.label(), "ALLOW");
    assert_eq!(FirewallAction::Deny.label(), "DENY");
    assert_eq!(
        FirewallAction::Unspecified.label(),
        "",
        "an underivable verdict contributes no word, so the column dashes"
    );
}

/// Direction labels and their round trip: the routing-mode pattern applied to
/// the other enum the VPC slice reads verbatim off the wire.
///
/// `Unrecognized` carries the raw string, so a direction Google adds later
/// shows up on screen as itself rather than as a blank or as whichever of the
/// two values this build happens to know.
#[test]
fn firewall_direction_labels_render_and_carry_the_unrecognised_verbatim() {
    assert_eq!(FirewallDirection::Ingress.label(), "INGRESS");
    assert_eq!(FirewallDirection::Egress.label(), "EGRESS");
    assert_eq!(
        FirewallDirection::Unspecified.label(),
        "",
        "an unreported direction contributes no word, so the column dashes"
    );
    assert_eq!(
        FirewallDirection::Unrecognized("SIDEWAYS".to_string()).label(),
        "SIDEWAYS"
    );

    assert_eq!(
        FirewallDirection::from_api(None),
        FirewallDirection::Unspecified
    );
    assert_eq!(
        FirewallDirection::from_api(Some("")),
        FirewallDirection::Unspecified
    );
    assert_eq!(
        FirewallDirection::from_api(Some("INGRESS")),
        FirewallDirection::Ingress
    );
    assert_eq!(
        FirewallDirection::from_api(Some("EGRESS")),
        FirewallDirection::Egress
    );
    assert_eq!(
        FirewallDirection::from_api(Some("SIDEWAYS")),
        FirewallDirection::Unrecognized("SIDEWAYS".to_string())
    );
}

/// A value the API added after this build renders as a readable label in every
/// status and state column, never as the Rust value that carries it.
///
/// Walks every enum whose `label()` paints a status or state cell. Each is
/// handed a constant no build knows, and each must spell it the way the labels
/// beside it are spelled. An empty value must not paint a blank cell either: a
/// blank status reads as "nothing wrong".
#[test]
fn every_status_label_spells_an_unrecognised_value_readably() {
    use nuvem_domain::gcp::resource::{
        CloudRunServiceStatus, CloudSqlStatus, GkeClusterStatus, ResourceStatus,
    };
    let unknown = "MAINTENANCE_WINDOW";
    let labels: [(&str, String, String); 9] = [
        (
            "ResourceStatus",
            ResourceStatus::Unknown(unknown.into()).label().into_owned(),
            ResourceStatus::Unknown(String::new()).label().into_owned(),
        ),
        (
            "GkeClusterStatus",
            GkeClusterStatus::Unknown(unknown.into())
                .label()
                .into_owned(),
            GkeClusterStatus::Unknown(String::new())
                .label()
                .into_owned(),
        ),
        (
            "CloudRunServiceStatus",
            CloudRunServiceStatus::Unknown(unknown.into())
                .label()
                .into_owned(),
            CloudRunServiceStatus::Unknown(String::new())
                .label()
                .into_owned(),
        ),
        (
            "CloudSqlStatus",
            CloudSqlStatus::Unknown(unknown.into()).label().into_owned(),
            CloudSqlStatus::Unknown(String::new()).label().into_owned(),
        ),
        (
            "CloudFunctionState",
            CloudFunctionState::Unrecognized(unknown.into())
                .label()
                .into_owned(),
            CloudFunctionState::Unrecognized(String::new())
                .label()
                .into_owned(),
        ),
        (
            "CloudFunctionEnvironment",
            CloudFunctionEnvironment::Unrecognized(unknown.into())
                .label()
                .into_owned(),
            CloudFunctionEnvironment::Unrecognized(String::new())
                .label()
                .into_owned(),
        ),
        (
            "DiskStatus",
            DiskStatus::Unrecognized(unknown.into())
                .label()
                .into_owned(),
            DiskStatus::Unrecognized(String::new()).label().into_owned(),
        ),
        (
            "PubSubSubscriptionState",
            PubSubSubscriptionState::Unrecognized(unknown.into())
                .label()
                .into_owned(),
            PubSubSubscriptionState::Unrecognized(String::new())
                .label()
                .into_owned(),
        ),
        (
            "SecretVersionState",
            SecretVersionState::Unknown(unknown.into())
                .label()
                .into_owned(),
            SecretVersionState::Unknown(String::new())
                .label()
                .into_owned(),
        ),
    ];
    for (name, label, empty) in labels {
        assert_eq!(label, "Maintenance window", "{name}");
        assert_eq!(empty, "Unknown", "{name}: an empty value");
    }
}

#[test]
fn readable_api_value_respells_constants_and_keeps_prose() {
    use nuvem_domain::gcp::resource::readable_api_value;
    for (raw, expected) in [
        ("MAINTENANCE_WINDOW", "Maintenance window"),
        ("PROVISIONING", "Provisioning"),
        ("GEN_3", "Gen 3"),
        ("DETACH-FAILED", "Detach failed"),
        ("  RUNNING  ", "Running"),
        ("", "Unknown"),
        ("   ", "Unknown"),
        // Already prose: the API's own words, kept as they came.
        ("Unknown", "Unknown"),
        ("Waiting for quota", "Waiting for quota"),
    ] {
        assert_eq!(readable_api_value(raw), expected, "{raw:?}");
    }
}
