//! Building the expected `RememberedViews` payload for effect assertions.
//!
//! Every service change now also emits `Effect::SaveRememberedViews`,
//! so the exact-effect assertions that surround a service switch have to name
//! the map that save carries. The payload has no public constructor beyond
//! `set`, and spelling that out at each site would bury the load effect the
//! test is actually about.

use nuvem::app::state::remembered_view::RememberedViews;
use nuvem_domain::gcp::configuration::GcloudConfigurationName;
use nuvem_domain::gcp::resource::ResourceServiceKind;

/// The map holding exactly one entry: `configuration` remembers `kind`.
pub(crate) fn remembered(configuration: &str, kind: ResourceServiceKind) -> RememberedViews {
    let mut views = RememberedViews::default();
    views.set(GcloudConfigurationName::new(configuration), kind);
    views
}
