//! Which resources service the user last had open in each gcloud
//! configuration — the payload behind eager saving.
//!
//! Two things about the representation carry weight.
//!
//! The map is keyed by [`GcloudConfigurationName`] rather than by
//! `GcloudContext`. A context also carries account, project, region and zone,
//! so keying on the whole of it would mint a fresh key every time the user
//! changes any of those properties — the feature would forget precisely when
//! the user is working most actively. The configuration name is the stable
//! user-facing identity, and it is what `--configuration` puts on the argv.
//!
//! The value is stored as the raw `String` from the file, converted at the
//! [`RememberedViews::get`] / [`RememberedViews::set`] boundary, rather than as
//! a [`ResourceServiceKind`]. That is what makes a name this build does not
//! recognise — a service a newer nuvem added — read as *absent* rather than
//! fail the whole file's deserialization, and it is also what lets this build
//! write the map back without destroying that entry. Storing the enum would
//! need a custom `Deserialize` to get the first property and could not have
//! the second at all.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use nuvem_domain::gcp::configuration::GcloudConfigurationName;
use nuvem_domain::gcp::resource::ResourceServiceKind;

use crate::config::{config_service_id, resolve_service_kind};

/// The remembered service per configuration.
///
/// A named field rather than a newtype so the payload serializes as a
/// `[data.views]` table under the state writer's `data` envelope key, leaving
/// room for a second kind of remembered state beside it later.
///
/// `BTreeMap` rather than `HashMap`: key order is then deterministic, so
/// re-saving an unchanged map produces byte-identical TOML instead of churning
/// the file.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RememberedViews {
    #[serde(default)]
    views: BTreeMap<GcloudConfigurationName, String>,
}

impl RememberedViews {
    /// The service remembered for `name`, or `None` when there is no entry or
    /// the stored name is one this build does not recognise.
    pub fn get(&self, name: &GcloudConfigurationName) -> Option<ResourceServiceKind> {
        self.views
            .get(name)
            .map(String::as_str)
            .and_then(resolve_service_kind)
    }

    /// Records `kind` as `name`'s remembered service, replacing any previous.
    pub fn set(&mut self, name: GcloudConfigurationName, kind: ResourceServiceKind) {
        self.views.insert(name, config_service_id(kind).to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(raw: &str) -> GcloudConfigurationName {
        GcloudConfigurationName::new(raw)
    }

    #[test]
    fn set_then_get_round_trips_a_service() {
        let mut views = RememberedViews::default();
        views.set(name("dev"), ResourceServiceKind::CloudSql);

        assert_eq!(views.get(&name("dev")), Some(ResourceServiceKind::CloudSql));
    }

    #[test]
    fn an_unset_configuration_has_no_remembered_service() {
        let views = RememberedViews::default();

        assert_eq!(views.get(&name("dev")), None);
    }

    #[test]
    fn set_replaces_the_previous_service_for_that_configuration() {
        let mut views = RememberedViews::default();
        views.set(name("dev"), ResourceServiceKind::CloudSql);
        views.set(name("dev"), ResourceServiceKind::GkeClusters);

        assert_eq!(
            views.get(&name("dev")),
            Some(ResourceServiceKind::GkeClusters)
        );
    }

    /// Every variant must survive the disk vocabulary. Driven off `ALL` rather
    /// than a hand-written list so a service added later cannot be writable
    /// but unreadable: the new variant joins this test automatically.
    #[test]
    fn every_service_kind_round_trips_through_the_config_vocabulary() {
        for kind in ResourceServiceKind::ALL {
            let mut views = RememberedViews::default();
            views.set(name("dev"), kind);

            assert_eq!(
                views.get(&name("dev")),
                Some(kind),
                "{kind:?} did not survive a set/get round trip"
            );
        }
    }

    /// A newer nuvem wrote a service this build does not have. The map still
    /// loads, this entry reads as absent, and its siblings are unaffected.
    ///
    /// The unreadable name has to be one no build resolves, and that is a
    /// moving target: this fixture said `"bigquery"` until nuvem shipped a
    /// BigQuery datasets service, at which point the name resolved and the
    /// example stopped being an example. `"spanner"` is the replacement
    /// because nothing in the registry answers to it — not a label, not an
    /// alias — and the next service that makes it resolve must move this
    /// fixture again rather than relax the assertion.
    #[test]
    fn an_unreadable_service_name_reads_as_absent_without_affecting_siblings() {
        let toml = "\
[views]
dev = \"spanner\"
prod = \"compute\"
";
        let views: RememberedViews = toml::from_str(toml).expect("map must still deserialize");

        assert_eq!(views.get(&name("dev")), None);
        assert_eq!(
            views.get(&name("prod")),
            Some(ResourceServiceKind::ComputeInstances)
        );
    }

    /// And the unreadable entry must survive being saved by this build, so
    /// running an older nuvem once does not destroy a newer one's memory.
    #[test]
    fn an_unreadable_service_name_survives_a_serialize_round_trip() {
        let toml = "\
[views]
dev = \"spanner\"
";
        let views: RememberedViews = toml::from_str(toml).expect("deserialize");
        let rendered = toml::to_string(&views).expect("serialize");
        let reloaded: RememberedViews = toml::from_str(&rendered).expect("deserialize again");

        assert_eq!(
            reloaded, views,
            "an entry this build cannot read must not be dropped when it saves"
        );
        assert!(
            rendered.contains("spanner"),
            "the unreadable name must still be on disk, got:\n{rendered}"
        );
    }
}
