//! Launching k9s against a GKE cluster, planned as data.
//!
//! Two decisions that would otherwise be made at the spawn site: which kube
//! context name a cluster maps to, and whether that context already exists so
//! the credentials fetch can be skipped. Both are total functions over plain
//! inputs, so the launch is decided — and testable — before anything runs.

use crate::gcp::configuration::{GcloudConfigurationName, GcloudContext};
use crate::gcp::resource::GkeCluster;

/// The kube context name `gcloud container clusters get-credentials` writes
/// for a cluster. Reproduced rather than read back, so a launch can name the
/// context it is about to create without a round trip.
pub fn kube_context_name(project: &str, location: &str, cluster: &str) -> String {
    format!("gke_{project}_{location}_{cluster}")
}

/// Everything `get-credentials` and k9s need in order to reach one cluster,
/// resolved by [`resolve_launch_target`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct K9sLaunchTarget {
    pub configuration: GcloudConfigurationName,
    pub project: String,
    pub location: String,
    pub cluster_name: String,
    pub kube_context: String,
}

/// The launch target for `cluster` under `context`, or `None` when there is no
/// launch to offer: no active project, an empty project, location or cluster
/// name, or a value `get-credentials` would read as an option rather than as
/// the thing it names.
pub fn resolve_launch_target(
    context: &GcloudContext,
    cluster: &GkeCluster,
) -> Option<K9sLaunchTarget> {
    let project = context.project.clone()?;
    if project.is_empty() || cluster.location.is_empty() || cluster.name.is_empty() {
        return None;
    }
    // All three reach `get-credentials`' argv, where the cluster name is
    // positional and the other two are flag values; any of them beginning with
    // `-` would be read as an option rather than as the thing it names. Refused
    // in the same breath as the empty check above, because the answer is the
    // same: there is no launch to offer.
    if crate::gcp::argv::option_like_value([
        project.as_str(),
        cluster.location.as_str(),
        cluster.name.as_str(),
    ])
    .is_some()
    {
        return None;
    }
    let kube_context = kube_context_name(&project, &cluster.location, &cluster.name);
    Some(K9sLaunchTarget {
        configuration: context.configuration.clone(),
        project,
        location: cluster.location.clone(),
        cluster_name: cluster.name.clone(),
        kube_context,
    })
}

impl K9sLaunchTarget {
    /// The `gcloud` arguments that write this cluster's kube context.
    pub fn credentials_argv(&self) -> Vec<String> {
        vec![
            "--configuration".into(),
            self.configuration.as_str().into(),
            "container".into(),
            "clusters".into(),
            "get-credentials".into(),
            self.cluster_name.clone(),
            "--location".into(),
            self.location.clone(),
            "--project".into(),
            self.project.clone(),
        ]
    }

    /// The `k9s` arguments that point it at that kube context.
    pub fn k9s_argv(&self) -> Vec<String> {
        vec!["--context".into(), self.kube_context.clone()]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialStep {
    ReuseExisting,
    RunGetCredentials,
}

/// Whether the kube context already exists and can be reused, or has to be
/// written first.
///
/// `None` for `existing_contexts` means the list could not be read, and is
/// treated as absent: running `get-credentials` again only rewrites the same
/// entry, while skipping it wrongly leaves k9s pointed at a context that is
/// not there.
pub fn resolve_credential_step(
    existing_contexts: Option<&[String]>,
    target: &K9sLaunchTarget,
) -> CredentialStep {
    match existing_contexts {
        Some(list) if list.iter().any(|name| name == &target.kube_context) => {
            CredentialStep::ReuseExisting
        }
        _ => CredentialStep::RunGetCredentials,
    }
}

/// A resolved target plus whether credentials must be fetched first — the
/// whole launch, decided before anything is spawned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct K9sLaunchPlan {
    pub target: K9sLaunchTarget,
    pub credential_step: CredentialStep,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gcp::configuration::{GcloudConfigurationName, GcloudContext};
    use crate::gcp::resource::{EndpointVisibility, GkeCluster, GkeClusterStatus};

    fn context(project: Option<&str>) -> GcloudContext {
        GcloudContext {
            configuration: GcloudConfigurationName::new("dev"),
            account: None,
            project: project.map(Into::into),
            region: None,
            zone: None,
        }
    }

    fn cluster(name: &str, location: &str) -> GkeCluster {
        GkeCluster {
            name: name.into(),
            location: location.into(),
            status: GkeClusterStatus::Running,
            current_master_version: None,
            current_node_count: None,
            endpoint_visibility: EndpointVisibility::Unknown,
        }
    }

    #[test]
    fn kube_context_name_uses_gke_convention() {
        assert_eq!(
            kube_context_name("proj", "us-central1-a", "web"),
            "gke_proj_us-central1-a_web"
        );
    }

    #[test]
    fn resolve_target_for_zonal_cluster() {
        let target =
            resolve_launch_target(&context(Some("proj")), &cluster("web", "us-central1-a"))
                .unwrap();
        assert_eq!(target.kube_context, "gke_proj_us-central1-a_web");
        assert_eq!(
            target.credentials_argv(),
            vec![
                "--configuration",
                "dev",
                "container",
                "clusters",
                "get-credentials",
                "web",
                "--location",
                "us-central1-a",
                "--project",
                "proj",
            ]
        );
        assert_eq!(
            target.k9s_argv(),
            vec!["--context", "gke_proj_us-central1-a_web"]
        );
    }

    #[test]
    fn resolve_target_for_regional_cluster() {
        let target =
            resolve_launch_target(&context(Some("proj")), &cluster("web", "us-central1")).unwrap();
        assert_eq!(target.kube_context, "gke_proj_us-central1_web");
        assert_eq!(target.credentials_argv()[7], "us-central1");
    }

    #[test]
    fn resolve_target_none_without_project() {
        assert!(resolve_launch_target(&context(None), &cluster("web", "us-central1-a")).is_none());
    }

    #[test]
    fn resolve_target_none_with_empty_location() {
        assert!(resolve_launch_target(&context(Some("proj")), &cluster("web", "")).is_none());
    }

    #[test]
    fn credential_step_reuses_when_context_present() {
        let target =
            resolve_launch_target(&context(Some("proj")), &cluster("web", "us-central1-a"))
                .unwrap();
        let existing = vec![
            "other".to_string(),
            "gke_proj_us-central1-a_web".to_string(),
        ];
        assert_eq!(
            resolve_credential_step(Some(&existing), &target),
            CredentialStep::ReuseExisting
        );
    }

    #[test]
    fn credential_step_runs_get_credentials_when_absent_or_unknown() {
        let target =
            resolve_launch_target(&context(Some("proj")), &cluster("web", "us-central1-a"))
                .unwrap();
        assert_eq!(
            resolve_credential_step(Some(&["other".to_string()]), &target),
            CredentialStep::RunGetCredentials
        );
        assert_eq!(
            resolve_credential_step(None, &target),
            CredentialStep::RunGetCredentials
        );
        assert_eq!(
            resolve_credential_step(Some(&[]), &target),
            CredentialStep::RunGetCredentials
        );
    }
}
