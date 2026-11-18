//! Which gcloud configuration a command runs under, and what that resolves to.
//!
//! Two types that are easy to confuse. [`GcloudConfiguration`] is a named
//! configuration exactly as `gcloud config configurations list` reports it,
//! activeness included. [`GcloudContext`] is what a command actually runs
//! under — the configuration it names plus the account, project, region and
//! zone that configuration resolved to.

use serde::{Deserialize, Serialize};

/// The name of a gcloud configuration, as `--configuration` takes it.
///
/// A newtype rather than a bare `String` so it cannot be confused with the
/// account, project, region or zone names it travels beside — every one of
/// which is also an unqualified string in the same struct.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct GcloudConfigurationName(String);

impl GcloudConfigurationName {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The name as gcloud spells it, for interpolation into an argv.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for GcloudConfigurationName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// One named gcloud configuration, as `gcloud config configurations list`
/// reports it, including whether it is the active one.
///
/// These are the settings the configuration *holds*. For what a command
/// running under it sees, use [`GcloudConfiguration::context`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GcloudConfiguration {
    pub name: GcloudConfigurationName,
    pub is_active: bool,
    pub account: Option<String>,
    pub project: Option<String>,
    pub region: Option<String>,
    pub zone: Option<String>,
}

/// The settings a command actually runs under: the configuration it names,
/// plus the account, project, region and zone that configuration resolved to.
///
/// Distinct from [`GcloudConfiguration`] in carrying no active flag. Every
/// command nuvem spawns passes `--configuration` explicitly, so which
/// configuration gcloud would have picked on its own is not part of the
/// context anything runs in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GcloudContext {
    pub configuration: GcloudConfigurationName,
    pub account: Option<String>,
    pub project: Option<String>,
    pub region: Option<String>,
    pub zone: Option<String>,
}

impl GcloudConfiguration {
    /// What a command running under this configuration would see.
    pub fn context(&self) -> GcloudContext {
        GcloudContext {
            configuration: self.name.clone(),
            account: self.account.clone(),
            project: self.project.clone(),
            region: self.region.clone(),
            zone: self.zone.clone(),
        }
    }
}
