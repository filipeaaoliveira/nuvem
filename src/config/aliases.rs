use std::collections::BTreeMap;

use super::resolve_service_id;
use crate::app::service_registry::{self, ServiceId};
use crate::config::problems::{ConfigProblemSource, ConfigProblems};

/// User-defined service aliases: extra names (beyond the built-in labels and
/// aliases in [`crate::app::service_registry`]) that resolve to an existing
/// registered service. Parsed from the `[aliases]` table of the config file
/// and merged into command-palette / service-navigator matching.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AliasTable {
    map: BTreeMap<String, ServiceId>,
}

impl AliasTable {
    /// An alias table with no entries (the default when no config file, or
    /// no `[aliases]` table, is present).
    pub fn empty() -> Self {
        Self::default()
    }

    /// Look up an alias (case-insensitively) and return the service it
    /// resolves to, if any.
    pub fn get(&self, alias: &str) -> Option<ServiceId> {
        self.map.get(&alias.to_ascii_lowercase()).copied()
    }

    /// Iterate over the validated `(alias, service)` entries.
    pub fn iter(&self) -> impl Iterator<Item = (&str, ServiceId)> + '_ {
        self.map.iter().map(|(alias, id)| (alias.as_str(), *id))
    }

    /// Parse and validate the raw `alias -> service` strings from the
    /// `[aliases]` config table. Each invalid entry is dropped and produces
    /// a warning; the remaining valid entries are kept.
    ///
    /// Validation rules:
    /// - the value must resolve to a known service (lenient `gce`/`gke`
    ///   matching, or a built-in service label/alias); an unknown value is
    ///   dropped with a warning.
    /// - the alias key must be non-empty once trimmed and lowercased.
    /// - the alias must not collide with a built-in service alias that maps
    ///   to a *different* service (a redefinition mapping to the *same*
    ///   service is harmless and kept).
    pub fn from_file(raw: &BTreeMap<String, String>, problems: &mut ConfigProblems) -> Self {
        let mut map = BTreeMap::new();

        for (key, value) in raw {
            let alias = key.trim().to_ascii_lowercase();
            if alias.is_empty() {
                problems.push(
                    ConfigProblemSource::Aliases,
                    format!("aliases: empty alias name (value {value:?}); skipping"),
                );
                continue;
            }

            let Some(service_id) = resolve_service_id(value) else {
                problems.push(
                    ConfigProblemSource::Aliases,
                    format!("aliases.{key}: unknown service {value:?}; skipping"),
                );
                continue;
            };

            if let Some(builtin_id) = builtin_alias_service(&alias)
                && builtin_id != service_id
            {
                problems.push(
                        ConfigProblemSource::Aliases,
                        format!(
                            "aliases.{key}: conflicts with a built-in alias for a different service; skipping"
                        ),
                    );
                continue;
            }

            map.insert(alias, service_id);
        }

        Self { map }
    }
}

/// Returns the service a *built-in* alias maps to, if `alias` matches one
/// exactly (case-insensitively).
fn builtin_alias_service(alias: &str) -> Option<ServiceId> {
    service_registry::service_descriptors()
        .iter()
        .find_map(|descriptor| {
            descriptor
                .aliases
                .iter()
                .any(|candidate| candidate.eq_ignore_ascii_case(alias))
                .then_some(descriptor.id)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn empty_alias_name_is_dropped_with_warning() {
        let (t, w) = table(&[("  ", "gce")]);
        assert_eq!(w.len(), 1);
        assert!(t.iter().next().is_none());
    }

    #[test]
    fn alias_lookup_is_case_insensitive() {
        let (t, w) = table(&[("VMs", "gce")]);
        assert!(w.is_empty());
        assert_eq!(t.get("vms"), Some(ServiceId::ComputeInstances));
        assert_eq!(t.get("VMS"), Some(ServiceId::ComputeInstances));
    }

    #[test]
    fn alias_matching_a_builtin_alias_for_a_different_service_is_dropped() {
        // "gke" is a built-in alias for GkeClusters; redefining it to point
        // at GCE is a conflicting override and must be dropped.
        let (t, w) = table(&[("gke", "gce")]);
        assert_eq!(w.len(), 1);
        assert_eq!(t.get("gke"), None);
    }

    #[test]
    fn alias_matching_a_builtin_alias_for_the_same_service_is_kept() {
        let (t, w) = table(&[("gce", "gce")]);
        assert!(w.is_empty());
        assert_eq!(t.get("gce"), Some(ServiceId::ComputeInstances));
    }

    #[test]
    fn alias_values_name_every_service_the_same_way_columns_blocks_do() {
        // `[aliases]` values and `[columns.<service>]` keys share one
        // vocabulary (`super::resolve_service_id`), so every service is
        // nameable from both.
        let (t, w) = table(&[
            ("a", "compute"),
            ("b", "gke"),
            ("c", "cloudsql"),
            ("d", "cloud_run"),
            ("e", "storage"),
        ]);
        assert!(w.is_empty(), "unexpected warnings: {w:?}");
        assert_eq!(t.get("a"), Some(ServiceId::ComputeInstances));
        assert_eq!(t.get("b"), Some(ServiceId::GkeClusters));
        assert_eq!(t.get("c"), Some(ServiceId::CloudSql));
        assert_eq!(t.get("d"), Some(ServiceId::CloudRun));
        assert_eq!(t.get("e"), Some(ServiceId::CloudStorage));
    }

    #[test]
    fn other_invalid_entries_are_dropped_while_valid_ones_are_kept() {
        let (t, w) = table(&[("vms", "gce"), ("bogus", "nope")]);
        assert_eq!(w.len(), 1);
        assert_eq!(t.get("vms"), Some(ServiceId::ComputeInstances));
        assert_eq!(t.get("bogus"), None);
    }
}
