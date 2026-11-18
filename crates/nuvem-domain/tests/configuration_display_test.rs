//! Pins that a configuration name displays as itself — the `Display` impl is
//! what every log line and picker row shows.

use nuvem_domain::gcp::configuration::GcloudConfigurationName;

#[test]
fn configuration_name_displays_as_itself() {
    let name = GcloudConfigurationName::new("staging-eu");
    assert_eq!(format!("{name}"), "staging-eu");
    assert_eq!(name.as_str(), "staging-eu");
}
