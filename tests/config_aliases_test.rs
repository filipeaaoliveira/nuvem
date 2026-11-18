use nuvem::app::service_registry::{ServiceId, matching_service_ids_with_aliases};
use nuvem::config::aliases::AliasTable;
use nuvem::config::problems::ConfigProblems;
use std::collections::BTreeMap;

fn table(pairs: &[(&str, &str)]) -> (AliasTable, Vec<String>) {
    let raw: BTreeMap<String, String> = pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let mut problems = ConfigProblems::default();
    let table = AliasTable::from_file(&raw, &mut problems);
    (table, problems.into_messages())
}

#[test]
fn valid_alias_resolves_to_service() {
    let (t, w) = table(&[("vms", "gce"), ("k8s", "gke")]);
    assert!(w.is_empty());
    assert_eq!(t.get("vms"), Some(ServiceId::ComputeInstances));
    assert_eq!(t.get("k8s"), Some(ServiceId::GkeClusters));
}

#[test]
fn unknown_service_value_is_dropped_with_warning() {
    let (t, w) = table(&[("foo", "not-a-service")]);
    assert_eq!(t.get("foo"), None);
    assert_eq!(w.len(), 1);
}

#[test]
fn user_alias_participates_in_matching() {
    let (t, _) = table(&[("vms", "gce")]);
    let ids = matching_service_ids_with_aliases("vms", &t);
    assert!(ids.contains(&ServiceId::ComputeInstances));
}
