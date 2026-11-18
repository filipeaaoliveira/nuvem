//! A Cloud Resource Manager project, as `gcloud projects list` reports it.

/// A project the active account can see.
///
/// `lifecycle_state` is the API's own string (`ACTIVE`, `DELETE_REQUESTED`)
/// rather than a bool: a project inside its deletion window still lists, and
/// is worth telling apart from a live one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GcloudProject {
    pub project_id: String,
    pub name: String,
    pub lifecycle_state: Option<String>,
}
