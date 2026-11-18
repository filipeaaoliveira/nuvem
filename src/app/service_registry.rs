use nuvem_domain::gcp::resource::ResourceServiceKind;

use super::effect::Effect;
use nuvem_domain::gcp::configuration::GcloudContext;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ServiceId {
    ComputeInstances,
    GkeClusters,
    CloudSql,
    CloudStorage,
    CloudRun,
    SecretManagerSecrets,
    CloudFunctions,
    PersistentDisks,
    PubSubTopics,
    PubSubSubscriptions,
    BigQueryDatasets,
    IamServiceAccounts,
    IamPolicy,
    VpcNetworks,
    VpcSubnets,
    FirewallRules,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceAvailability {
    Available,
    Unavailable(&'static str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceTarget {
    Resource(ResourceServiceKind),
}

impl ServiceTarget {
    pub fn resource_kind(self) -> Option<ResourceServiceKind> {
        match self {
            Self::Resource(kind) => Some(kind),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServiceDescriptor {
    pub id: ServiceId,
    pub label: &'static str,
    pub category: &'static str,
    pub aliases: &'static [&'static str],
    pub default_sort: u16,
    pub availability: ServiceAvailability,
    pub target: Option<ServiceTarget>,
}

impl ServiceDescriptor {
    pub fn is_available(self) -> bool {
        matches!(self.availability, ServiceAvailability::Available)
    }

    pub fn unavailable_reason(self) -> Option<&'static str> {
        match self.availability {
            ServiceAvailability::Available => None,
            ServiceAvailability::Unavailable(reason) => Some(reason),
        }
    }
}

pub const SERVICE_REGISTRY: [ServiceDescriptor; 16] = [
    ServiceDescriptor {
        id: ServiceId::BigQueryDatasets,
        label: "BigQuery datasets",
        // "Analytics": a dataset is a warehouse container, not a database
        // instance an operator administers, so filing it under Databases
        // beside Cloud SQL would make a search for "database" name a service
        // that has no instance, no tier and no failover.
        category: "Analytics",
        // `bq` earns its place: it is the binary an operator runs against
        // this surface, so it is the word already in their fingers.
        // `bigquery-datasets` is the canonical config spelling
        // (`config_service_id`), so a rule's `from` resolves verbatim.
        aliases: &["bigquery", "bq", "datasets", "bigquery-datasets"],
        default_sort: 10,
        availability: ServiceAvailability::Available,
        target: Some(ServiceTarget::Resource(
            ResourceServiceKind::BigQueryDatasets,
        )),
    },
    ServiceDescriptor {
        id: ServiceId::ComputeInstances,
        label: "GCE instances",
        category: "Compute",
        aliases: &["gce", "compute engine", "instances"],
        default_sort: 10,
        availability: ServiceAvailability::Available,
        target: Some(ServiceTarget::Resource(
            ResourceServiceKind::ComputeInstances,
        )),
    },
    ServiceDescriptor {
        id: ServiceId::GkeClusters,
        label: "GKE clusters",
        category: "Containers",
        aliases: &["gke", "kubernetes", "clusters"],
        default_sort: 20,
        availability: ServiceAvailability::Available,
        target: Some(ServiceTarget::Resource(ResourceServiceKind::GkeClusters)),
    },
    ServiceDescriptor {
        id: ServiceId::CloudSql,
        label: "Cloud SQL",
        category: "Databases",
        aliases: &["cloud sql", "sql", "database", "postgres", "mysql"],
        default_sort: 40,
        availability: ServiceAvailability::Available,
        target: Some(ServiceTarget::Resource(ResourceServiceKind::CloudSql)),
    },
    ServiceDescriptor {
        id: ServiceId::PubSubTopics,
        label: "Pub/Sub topics",
        category: "Messaging",
        aliases: &[
            "pubsub",
            "pub/sub",
            "topics",
            "pubsub topics",
            "pubsub-topics",
        ],
        default_sort: 20,
        availability: ServiceAvailability::Available,
        target: Some(ServiceTarget::Resource(ResourceServiceKind::PubSubTopics)),
    },
    ServiceDescriptor {
        id: ServiceId::PubSubSubscriptions,
        label: "Pub/Sub subscriptions",
        category: "Messaging",
        // `subs` earns its place: it is the header the topics table uses for
        // its count column, so it is the word an operator already associates
        // with this resource.
        aliases: &[
            "subscriptions",
            "subs",
            "pubsub subscriptions",
            "pubsub-subscriptions",
        ],
        default_sort: 21,
        availability: ServiceAvailability::Available,
        target: Some(ServiceTarget::Resource(
            ResourceServiceKind::PubSubSubscriptions,
        )),
    },
    ServiceDescriptor {
        id: ServiceId::VpcNetworks,
        label: "VPC networks",
        // "Networking", a category of its own rather than "Compute": a VPC
        // network outlives every instance in it and is the surface an operator
        // reaches for when the question is connectivity rather than capacity.
        // Filing it under Compute would also make a search for "compute" name
        // two services, and that word means GCE here. The category sorts
        // between "Messaging" and "Security", which is where this descriptor
        // has to sit — the registry is declared in the order the search
        // returns.
        category: "Networking",
        // `vpc` is what the console and every GCP conversation call the
        // surface, and `vpc-networks` is the canonical config spelling
        // (`config_service_id`), so a rule's `from` resolves verbatim.
        aliases: &["vpc", "networks", "vpc networks", "vpc-networks"],
        default_sort: 10,
        availability: ServiceAvailability::Available,
        target: Some(ServiceTarget::Resource(ResourceServiceKind::VpcNetworks)),
    },
    ServiceDescriptor {
        id: ServiceId::VpcSubnets,
        label: "VPC subnets",
        // Same category as the networks table, and `default_sort` 11 puts it
        // immediately after — an operator looking at a network reaches for its
        // subnets next, and the two read as one surface in the navigator.
        category: "Networking",
        // `subnets` and `subnetworks` both: the console says the first, the
        // API and the gcloud subcommand say the second, and an operator will
        // type whichever they last read. `vpc-subnets` is the canonical config
        // spelling (`config_service_id`), so a rule's `from` resolves verbatim.
        aliases: &["subnets", "subnetworks", "vpc subnets", "vpc-subnets"],
        default_sort: 11,
        availability: ServiceAvailability::Available,
        target: Some(ServiceTarget::Resource(ResourceServiceKind::VpcSubnets)),
    },
    ServiceDescriptor {
        id: ServiceId::FirewallRules,
        label: "Firewall rules",
        // The third "Networking" table, and `default_sort` 12 keeps the three
        // in the order an operator reads them: what the network is, how it is
        // carved up, and what may cross it.
        category: "Networking",
        // `firewall` singular and `fw` both: the gcloud subcommand is
        // `firewall-rules`, the console says "Firewall", and `fw` is what an
        // operator types. `firewall-rules` is the canonical config spelling
        // (`config_service_id`), so a rule's `from` resolves verbatim.
        aliases: &["firewall", "firewalls", "fw", "firewall-rules"],
        default_sort: 12,
        availability: ServiceAvailability::Available,
        target: Some(ServiceTarget::Resource(ResourceServiceKind::FirewallRules)),
    },
    ServiceDescriptor {
        id: ServiceId::SecretManagerSecrets,
        label: "Secret Manager",
        category: "Security",
        aliases: &["secret", "secrets", "secret manager", "sm"],
        default_sort: 10,
        availability: ServiceAvailability::Available,
        target: Some(ServiceTarget::Resource(
            ResourceServiceKind::SecretManagerSecrets,
        )),
    },
    ServiceDescriptor {
        id: ServiceId::IamServiceAccounts,
        label: "Service accounts",
        // "Security", joining Secret Manager: a service account is an
        // identity, and the reason an operator opens this table is an access
        // review, not a workload question.
        category: "Security",
        // `iam` is how the console and gcloud both file the surface, and
        // `service-accounts` is the canonical config spelling
        // (`config_service_id`), so a rule's `from` resolves verbatim.
        aliases: &["iam", "sa", "service accounts", "service-accounts"],
        default_sort: 20,
        availability: ServiceAvailability::Available,
        target: Some(ServiceTarget::Resource(
            ResourceServiceKind::IamServiceAccounts,
        )),
    },
    ServiceDescriptor {
        id: ServiceId::IamPolicy,
        label: "IAM policy",
        // "Security", beside the service accounts table it answers the other
        // half of: who holds which role on this project.
        category: "Security",
        // `iam-policy` is the canonical config spelling (`config_service_id`),
        // so a rule's `from` resolves verbatim; the rest are what an operator
        // reaches for.
        aliases: &["iam policy", "iam-policy", "policy", "bindings", "roles"],
        default_sort: 21,
        availability: ServiceAvailability::Available,
        target: Some(ServiceTarget::Resource(ResourceServiceKind::IamPolicy)),
    },
    ServiceDescriptor {
        id: ServiceId::CloudRun,
        label: "Cloud Run",
        category: "Serverless",
        aliases: &["run", "serverless"],
        default_sort: 30,
        availability: ServiceAvailability::Available,
        target: Some(ServiceTarget::Resource(
            ResourceServiceKind::CloudRunServices,
        )),
    },
    ServiceDescriptor {
        id: ServiceId::CloudFunctions,
        label: "Cloud Functions",
        category: "Serverless",
        aliases: &["functions", "gcf", "cloud functions"],
        default_sort: 35,
        availability: ServiceAvailability::Available,
        target: Some(ServiceTarget::Resource(ResourceServiceKind::CloudFunctions)),
    },
    ServiceDescriptor {
        id: ServiceId::PersistentDisks,
        label: "Persistent disks",
        // "Storage", not "Compute": a persistent disk is block storage that
        // outlives the instance it was attached to, which is the whole reason
        // an orphan is billable. Filing it under Compute would also make a
        // search for "compute" name two services, and that word means GCE.
        category: "Storage",
        // `pd` is what the console and every GCP conversation calls them, and
        // `orphans` is the question an operator actually opens this service to
        // ask, so it resolves the service by the reason for reaching it.
        aliases: &["disks", "pd", "persistent disks", "orphans"],
        default_sort: 37,
        availability: ServiceAvailability::Available,
        target: Some(ServiceTarget::Resource(
            ResourceServiceKind::PersistentDisks,
        )),
    },
    ServiceDescriptor {
        id: ServiceId::CloudStorage,
        label: "Cloud Storage",
        category: "Storage",
        aliases: &["gcs", "storage", "bucket", "buckets"],
        default_sort: 40,
        availability: ServiceAvailability::Available,
        target: Some(ServiceTarget::Resource(ResourceServiceKind::CloudStorage)),
    },
];

pub fn service_descriptors() -> &'static [ServiceDescriptor] {
    &SERVICE_REGISTRY
}

pub fn service_id_for_resource(kind: ResourceServiceKind) -> Option<ServiceId> {
    SERVICE_REGISTRY
        .iter()
        .find(|service| service.target.and_then(ServiceTarget::resource_kind) == Some(kind))
        .map(|service| service.id)
}

pub fn descriptor(id: ServiceId) -> &'static ServiceDescriptor {
    SERVICE_REGISTRY
        .iter()
        .find(|service| service.id == id)
        .expect("every ServiceId must have a registry descriptor")
}

pub fn matching_service_ids(query: &str) -> Vec<ServiceId> {
    let query = query.trim().to_lowercase();

    let mut matches = SERVICE_REGISTRY
        .iter()
        .filter(|service| {
            [service.label, service.category]
                .into_iter()
                .chain(service.aliases.iter().copied())
                .any(|value| value.to_lowercase().contains(&query))
        })
        .collect::<Vec<_>>();
    matches.sort_by(|left, right| {
        left.category
            .cmp(right.category)
            .then_with(|| left.default_sort.cmp(&right.default_sort))
            .then_with(|| left.label.cmp(right.label))
    });

    matches.into_iter().map(|service| service.id).collect()
}

/// Like [`matching_service_ids`], but also matches against a user-supplied
/// [`AliasTable`](crate::config::aliases::AliasTable). Built-in matches come
/// first (in their existing sort order); any additional services that only
/// match via a user alias are appended after, deduplicated.
pub fn matching_service_ids_with_aliases(
    query: &str,
    extra: &crate::config::aliases::AliasTable,
) -> Vec<ServiceId> {
    let mut ids = matching_service_ids(query);
    let query = query.trim().to_lowercase();

    for (alias, service_id) in extra.iter() {
        if alias.to_lowercase().contains(&query) && !ids.contains(&service_id) {
            ids.push(service_id);
        }
    }

    ids
}

/// The list-load effect for `id`.
///
/// `filter` is the applied filter to push as a server-side `--filter`. It must
/// be the applied filter of the `ResourceServiceKind` that `id` maps to — use
/// `reducer::resources::active_filter_for`, which reads exactly that service's
/// own filter. A filter belonging to another service would narrow the wrong
/// stored list; conversely, dropping a service's own filter silently disarms
/// it, because label mode narrows on the server only.
/// Cloud Run and Cloud Storage carry no `--filter` at all, so `filter` is
/// ignored for them.
pub fn load_effect(
    id: ServiceId,
    context: GcloudContext,
    filter: Option<String>,
) -> Option<Effect> {
    match descriptor(id).target {
        Some(ServiceTarget::Resource(ResourceServiceKind::ComputeInstances)) => {
            Some(Effect::RefreshComputeInstances { context, filter })
        }
        Some(ServiceTarget::Resource(ResourceServiceKind::GkeClusters)) => {
            Some(Effect::RefreshGkeClusters { context, filter })
        }
        Some(ServiceTarget::Resource(ResourceServiceKind::CloudSql)) => {
            Some(Effect::RefreshCloudSqlInstances { context, filter })
        }
        Some(ServiceTarget::Resource(ResourceServiceKind::CloudRunServices)) => {
            Some(Effect::RefreshCloudRunServices { context })
        }
        Some(ServiceTarget::Resource(ResourceServiceKind::CloudStorage)) => {
            Some(Effect::RefreshStorageBuckets { context })
        }
        Some(ServiceTarget::Resource(ResourceServiceKind::SecretManagerSecrets)) => {
            Some(Effect::RefreshSecrets { context, filter })
        }
        Some(ServiceTarget::Resource(ResourceServiceKind::CloudFunctions)) => {
            Some(Effect::RefreshCloudFunctions { context, filter })
        }
        Some(ServiceTarget::Resource(ResourceServiceKind::PersistentDisks)) => {
            Some(Effect::RefreshPersistentDisks { context, filter })
        }
        Some(ServiceTarget::Resource(ResourceServiceKind::PubSubTopics)) => {
            Some(Effect::RefreshPubSubTopics { context, filter })
        }
        Some(ServiceTarget::Resource(ResourceServiceKind::PubSubSubscriptions)) => {
            Some(Effect::RefreshPubSubSubscriptions { context, filter })
        }
        Some(ServiceTarget::Resource(ResourceServiceKind::VpcNetworks)) => {
            Some(Effect::RefreshVpcNetworks { context })
        }
        Some(ServiceTarget::Resource(ResourceServiceKind::VpcSubnets)) => {
            Some(Effect::RefreshVpcSubnets { context })
        }
        Some(ServiceTarget::Resource(ResourceServiceKind::FirewallRules)) => {
            Some(Effect::RefreshFirewallRules { context })
        }
        Some(ServiceTarget::Resource(ResourceServiceKind::IamServiceAccounts)) => {
            Some(Effect::RefreshServiceAccounts { context, filter })
        }
        // No filter: `bq ls` accepts no server-side filter expression, so
        // there is nothing to push down and the datasets table narrows wholly
        // client-side.
        Some(ServiceTarget::Resource(ResourceServiceKind::BigQueryDatasets)) => {
            Some(Effect::RefreshBigQueryDatasets { context })
        }
        // The policy read takes no filter: it fetches one object and flattens
        // it, so there is no list request to narrow.
        Some(ServiceTarget::Resource(ResourceServiceKind::IamPolicy)) => {
            Some(Effect::RefreshIamPolicy { context })
        }
        None => None,
    }
}
