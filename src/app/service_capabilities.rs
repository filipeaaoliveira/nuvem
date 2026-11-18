//! What a resource service actually answers, per key-bound capability.
//!
//! Five bindings in the resource catalogue are offered on every service and
//! answered by two or three: batch marking, the managed tunnel, the Cloud
//! Storage object browser's three keys, and Cloud Run's revision drill. The
//! reducer knows which — it branches on `active_service` and returns early for
//! the rest — but the help catalogue had nowhere to ask, so it taught all five
//! everywhere and the operator was told about keys their view cannot use.
//!
//! Each predicate is an exhaustive `match` rather than a list of the services
//! that qualify, so a new `ResourceServiceKind` is a compile error here instead
//! of a silent `false`. That is the half the compiler can enforce; the values
//! are checked against the reducer's own branches by
//! `tests/service_capability_parity_test.rs`, since agreeing with the compiler
//! is not the same as agreeing with the code that does the work.

use nuvem_domain::gcp::resource::ResourceServiceKind;

/// Whether rows of this service can be marked for a batch action (`Space`,
/// `V`, `\`, `U`).
///
/// Mirrors `ResourcesState::toggle_mark`, whose other arms are empty blocks.
pub fn supports_marks(kind: ResourceServiceKind) -> bool {
    match kind {
        ResourceServiceKind::ComputeInstances | ResourceServiceKind::CloudSql => true,
        ResourceServiceKind::GkeClusters
        | ResourceServiceKind::CloudRunServices
        | ResourceServiceKind::CloudStorage
        | ResourceServiceKind::SecretManagerSecrets
        | ResourceServiceKind::CloudFunctions
        | ResourceServiceKind::PersistentDisks
        | ResourceServiceKind::PubSubTopics
        | ResourceServiceKind::PubSubSubscriptions
        | ResourceServiceKind::BigQueryDatasets
        | ResourceServiceKind::IamServiceAccounts
        | ResourceServiceKind::IamPolicy
        | ResourceServiceKind::VpcNetworks
        | ResourceServiceKind::VpcSubnets
        | ResourceServiceKind::FirewallRules => false,
    }
}

/// Whether `t` can resolve a managed tunnel from a row of this service.
///
/// Mirrors `reducer::resources::request_tunnel`, whose remaining arms `return`.
pub fn supports_tunnel(kind: ResourceServiceKind) -> bool {
    match kind {
        ResourceServiceKind::ComputeInstances | ResourceServiceKind::CloudSql => true,
        ResourceServiceKind::GkeClusters
        | ResourceServiceKind::CloudRunServices
        | ResourceServiceKind::CloudStorage
        | ResourceServiceKind::SecretManagerSecrets
        | ResourceServiceKind::CloudFunctions
        | ResourceServiceKind::PersistentDisks
        | ResourceServiceKind::PubSubTopics
        | ResourceServiceKind::PubSubSubscriptions
        | ResourceServiceKind::BigQueryDatasets
        | ResourceServiceKind::IamServiceAccounts
        | ResourceServiceKind::IamPolicy
        | ResourceServiceKind::VpcNetworks
        | ResourceServiceKind::VpcSubnets
        | ResourceServiceKind::FirewallRules => false,
    }
}

/// Whether this service has the object browser, and with it `i`, `y` and `n`.
///
/// Mirrors the `active_service() != CloudStorage` early return that opens
/// `OpenBucketMetadata` and `NextPage` in the reducer.
pub fn browses_objects(kind: ResourceServiceKind) -> bool {
    match kind {
        ResourceServiceKind::CloudStorage => true,
        ResourceServiceKind::ComputeInstances
        | ResourceServiceKind::CloudSql
        | ResourceServiceKind::GkeClusters
        | ResourceServiceKind::CloudRunServices
        | ResourceServiceKind::SecretManagerSecrets
        | ResourceServiceKind::CloudFunctions
        | ResourceServiceKind::PersistentDisks
        | ResourceServiceKind::PubSubTopics
        | ResourceServiceKind::PubSubSubscriptions
        | ResourceServiceKind::BigQueryDatasets
        | ResourceServiceKind::IamServiceAccounts
        | ResourceServiceKind::IamPolicy
        | ResourceServiceKind::VpcNetworks
        | ResourceServiceKind::VpcSubnets
        | ResourceServiceKind::FirewallRules => false,
    }
}
