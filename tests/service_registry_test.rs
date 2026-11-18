use nuvem::app::Effect;
use nuvem::app::service_registry::{
    ServiceAvailability, ServiceId, descriptor, load_effect, matching_service_ids,
    service_descriptors, service_id_for_resource,
};
use nuvem_domain::gcp::configuration::{GcloudConfigurationName, GcloudContext};
use nuvem_domain::gcp::resource::ResourceServiceKind;

#[test]
fn registry_contains_sorted_available_services() {
    let ids = service_descriptors()
        .iter()
        .map(|service| service.id)
        .collect::<Vec<_>>();

    assert_eq!(
        ids,
        vec![
            // "Analytics" sorts ahead of every other category, so the
            // datasets table leads the registry.
            ServiceId::BigQueryDatasets,
            ServiceId::ComputeInstances,
            ServiceId::GkeClusters,
            ServiceId::CloudSql,
            // "Messaging" sorts between "Databases" and "Security".
            ServiceId::PubSubTopics,
            // Same category as the topics table, and `default_sort` 21 puts it
            // after.
            ServiceId::PubSubSubscriptions,
            // "Networking" sorts between "Messaging" and "Security".
            ServiceId::VpcNetworks,
            // Same category, and `default_sort` 11 puts it after.
            ServiceId::VpcSubnets,
            // Same category again, and `default_sort` 12 puts it last of the
            // three: a firewall rule is scoped by the network the other two
            // tables describe, so it reads after them.
            ServiceId::FirewallRules,
            // "Security" sorts between "Networking" and "Serverless", and the
            // registry is declared in the order the search returns.
            ServiceId::SecretManagerSecrets,
            // Same category as Secret Manager, and `default_sort` 20 puts it
            // after.
            ServiceId::IamServiceAccounts,
            // Same category again, and `default_sort` 21 puts it after the
            // service accounts table.
            ServiceId::IamPolicy,
            ServiceId::CloudRun,
            // Same category as Cloud Run, and `default_sort` 35 puts it after.
            ServiceId::CloudFunctions,
            // "Storage" sorts after "Serverless", and `default_sort` 37 puts
            // persistent disks ahead of Cloud Storage within it.
            ServiceId::PersistentDisks,
            ServiceId::CloudStorage
        ]
    );
    assert!(matches!(
        descriptor(ServiceId::ComputeInstances).availability,
        ServiceAvailability::Available
    ));
    assert!(matches!(
        descriptor(ServiceId::CloudRun).availability,
        ServiceAvailability::Available
    ));
    assert_eq!(descriptor(ServiceId::CloudRun).unavailable_reason(), None);
}

#[test]
fn service_search_matches_label_category_and_aliases_case_insensitively() {
    assert_eq!(
        matching_service_ids("gce"),
        vec![ServiceId::ComputeInstances]
    );
    assert_eq!(
        matching_service_ids("kubernetes"),
        vec![ServiceId::GkeClusters]
    );
    assert_eq!(
        matching_service_ids("compute"),
        vec![ServiceId::ComputeInstances]
    );
}

#[test]
fn service_search_empty_query_returns_every_descriptor_in_default_order() {
    assert_eq!(
        matching_service_ids(""),
        service_descriptors()
            .iter()
            .map(|service| service.id)
            .collect::<Vec<_>>()
    );
}

#[test]
fn service_search_orders_matches_by_category_then_default_sort_then_label() {
    let mut expected = service_descriptors()
        .iter()
        .filter(|service| {
            [service.label, service.category]
                .into_iter()
                .chain(service.aliases.iter().copied())
                .any(|value| value.to_lowercase().contains('e'))
        })
        .collect::<Vec<_>>();
    expected.sort_by(|left, right| {
        left.category
            .cmp(right.category)
            .then_with(|| left.default_sort.cmp(&right.default_sort))
            .then_with(|| left.label.cmp(right.label))
    });

    let expected_ids = expected
        .into_iter()
        .map(|service| service.id)
        .collect::<Vec<_>>();
    assert_eq!(matching_service_ids("e"), expected_ids);
}

#[test]
fn service_targets_and_availability_expose_domain_capabilities() {
    assert_eq!(
        descriptor(ServiceId::ComputeInstances)
            .target
            .expect("GCE should have a resource target")
            .resource_kind(),
        Some(ResourceServiceKind::ComputeInstances)
    );
    assert!(descriptor(ServiceId::GkeClusters).is_available());
    assert!(descriptor(ServiceId::CloudRun).is_available());
    assert_eq!(
        descriptor(ServiceId::CloudRun)
            .target
            .expect("Cloud Run should have a resource target")
            .resource_kind(),
        Some(ResourceServiceKind::CloudRunServices)
    );
}

#[test]
fn service_search_trims_query_before_matching() {
    assert_eq!(
        matching_service_ids("  SERVERLESS  "),
        vec![ServiceId::CloudRun, ServiceId::CloudFunctions]
    );
}

#[test]
fn cloud_sql_descriptor_is_available_and_targets_the_resource_kind() {
    let cloud_sql = descriptor(ServiceId::CloudSql);
    assert_eq!(cloud_sql.label, "Cloud SQL");
    assert_eq!(cloud_sql.category, "Databases");
    assert!(cloud_sql.is_available());
    assert_eq!(
        cloud_sql.target.and_then(|target| target.resource_kind()),
        Some(ResourceServiceKind::CloudSql)
    );
}

#[test]
fn cloud_sql_matches_its_aliases() {
    assert_eq!(matching_service_ids("cloud sql"), vec![ServiceId::CloudSql]);
    assert_eq!(matching_service_ids("postgres"), vec![ServiceId::CloudSql]);
    assert_eq!(matching_service_ids("mysql"), vec![ServiceId::CloudSql]);
}

#[test]
fn cloud_sql_load_effect_refreshes_instances() {
    let context = GcloudContext {
        configuration: GcloudConfigurationName::new("dev"),
        account: None,
        project: Some("dev-project".to_string()),
        region: None,
        zone: None,
    };
    assert_eq!(
        load_effect(ServiceId::CloudSql, context.clone(), None),
        Some(Effect::RefreshCloudSqlInstances {
            context: context.clone(),
            filter: None
        })
    );
    // A supplied filter is carried through verbatim; callers are responsible
    // for supplying one only when the loaded service is the active one.
    assert_eq!(
        load_effect(
            ServiceId::CloudSql,
            context.clone(),
            Some("-l env=prod".to_string())
        ),
        Some(Effect::RefreshCloudSqlInstances {
            context,
            filter: Some("-l env=prod".to_string())
        })
    );
}

/// Every resource service must resolve through the registry.
///
/// `ResourcesAction::SelectService` and the jump/history open path both reach
/// `load_effect` through `service_id_for_resource`, so a service the registry
/// does not map is not a compile error but a silent "select it and never load
/// it": the list opens empty and nothing is ever fetched into it. This is the
/// backstop the exhaustive match used to provide.
#[test]
fn every_resource_service_kind_resolves_to_a_registry_service_id() {
    for kind in ResourceServiceKind::ALL {
        let id = service_id_for_resource(kind);
        assert!(
            id.is_some(),
            "{kind:?} has no ServiceId, so selecting it would never load anything"
        );
        assert!(
            load_effect(
                id.unwrap(),
                GcloudContext {
                    configuration: GcloudConfigurationName::new("dev"),
                    account: None,
                    project: Some("dev-project".to_string()),
                    region: None,
                    zone: None,
                },
                None,
            )
            .is_some(),
            "{kind:?} resolves to a ServiceId that emits no list load"
        );
    }
}

/// Secret Manager's list load, and the filter it must carry.
///
/// `every_resource_service_kind_resolves_to_a_registry_service_id` proves only
/// that *some* effect comes back. This pins which one and with what: a label
/// filter narrows Secret Manager on the server only — labels are not on the
/// list row, so nothing client-side can narrow on them — and a dropped filter
/// therefore returns the whole project under a chip that says otherwise.
#[test]
fn secrets_load_effect_refreshes_secrets_and_carries_the_filter() {
    let context = GcloudContext {
        configuration: GcloudConfigurationName::new("dev"),
        account: None,
        project: Some("dev-project".to_string()),
        region: None,
        zone: None,
    };

    assert_eq!(
        load_effect(ServiceId::SecretManagerSecrets, context.clone(), None),
        Some(Effect::RefreshSecrets {
            context: context.clone(),
            filter: None
        })
    );
    assert_eq!(
        load_effect(
            ServiceId::SecretManagerSecrets,
            context.clone(),
            Some("-l env=prod".to_string())
        ),
        Some(Effect::RefreshSecrets {
            context,
            filter: Some("-l env=prod".to_string())
        })
    );
}

/// Every registered service that targets a resource emits *that* resource's
/// load, not merely some load.
///
/// The exhaustive walk above accepts any `Some(_)`, so a `load_effect` that
/// answered every id with one service's refresh would pass it while quietly
/// loading the wrong list into every table.
#[test]
fn each_resource_service_load_effect_names_its_own_resource_kind() {
    let context = GcloudContext {
        configuration: GcloudConfigurationName::new("dev"),
        account: None,
        project: Some("dev-project".to_string()),
        region: None,
        zone: None,
    };

    for kind in ResourceServiceKind::ALL {
        let id = service_id_for_resource(kind).expect("every resource kind maps to a service");
        let effect = load_effect(id, context.clone(), None).expect("every resource kind loads");
        let matches_kind = match kind {
            ResourceServiceKind::ComputeInstances => {
                matches!(effect, Effect::RefreshComputeInstances { .. })
            }
            ResourceServiceKind::GkeClusters => matches!(effect, Effect::RefreshGkeClusters { .. }),
            ResourceServiceKind::CloudSql => {
                matches!(effect, Effect::RefreshCloudSqlInstances { .. })
            }
            ResourceServiceKind::CloudRunServices => {
                matches!(effect, Effect::RefreshCloudRunServices { .. })
            }
            ResourceServiceKind::CloudStorage => {
                matches!(effect, Effect::RefreshStorageBuckets { .. })
            }
            ResourceServiceKind::PersistentDisks => {
                matches!(effect, Effect::RefreshPersistentDisks { .. })
            }
            ResourceServiceKind::SecretManagerSecrets => {
                matches!(effect, Effect::RefreshSecrets { .. })
            }
            ResourceServiceKind::CloudFunctions => {
                matches!(effect, Effect::RefreshCloudFunctions { .. })
            }
            ResourceServiceKind::PubSubTopics => {
                matches!(effect, Effect::RefreshPubSubTopics { .. })
            }
            ResourceServiceKind::PubSubSubscriptions => {
                matches!(effect, Effect::RefreshPubSubSubscriptions { .. })
            }
            ResourceServiceKind::IamServiceAccounts => {
                matches!(effect, Effect::RefreshServiceAccounts { .. })
            }
            ResourceServiceKind::IamPolicy => matches!(effect, Effect::RefreshIamPolicy { .. }),
            ResourceServiceKind::BigQueryDatasets => {
                matches!(effect, Effect::RefreshBigQueryDatasets { .. })
            }
            ResourceServiceKind::VpcNetworks => {
                matches!(effect, Effect::RefreshVpcNetworks { .. })
            }
            ResourceServiceKind::VpcSubnets => {
                matches!(effect, Effect::RefreshVpcSubnets { .. })
            }
            ResourceServiceKind::FirewallRules => {
                matches!(effect, Effect::RefreshFirewallRules { .. })
            }
        };
        assert!(
            matches_kind,
            "{kind:?} must load its own list, not {effect:?}"
        );
    }
}
