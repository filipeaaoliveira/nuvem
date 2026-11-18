//! IAM service accounts and the project's IAM policy, read only.
//!
//! # Two calls, two shapes, one module
//!
//! `iam service-accounts list` is an ordinary project-scoped list and returns
//! an array. `projects get-iam-policy` is not a list at all: it returns one
//! *policy object* whose `bindings` array is the thing worth rendering, and its
//! `--limit` — which gcloud offers on every command — would cap the number of
//! **policies** returned rather than the number of bindings, so passing it
//! would be a no-op at best. That asymmetry is why the two loads below do not
//! share a helper beyond `base_args`.
//!
//! The project is a *positional* argument to `get-iam-policy`, not the
//! `--project` flag every other service in this crate passes. A context with no
//! project therefore cannot issue the call at all, and says so
//! ([`IamLoadError::MissingProject`]) rather than shelling out to a command
//! that would fail with gcloud's own wording.
//!
//! # Neither load pushes a filter down
//!
//! There is deliberately no `FILTER_FIELDS` constant here, because there is no
//! call to `pushdown_expression` for one to feed: `/` narrows these two tables
//! entirely client-side.
//!
//! The never-drop-a-row invariant in `crate::services::resources::list_filter`
//! is a claim about *display* values, and both of these services have
//! searchable values present in no raw gcloud field — the accounts table
//! renders the disabled state as a word where the wire carries a JSON boolean,
//! and the policy table renders `Service account` where the wire carries a
//! `serviceAccount:` prefix inside a larger string. Pushing anything would mean
//! enumerating every one of those as an opaque label; not pushing degrades to
//! full client-side narrowing, which is the safe direction.
//!
//! Neither resource carries user labels at all, so there is no label field to
//! push either. `-l k=v` on these services therefore narrows not at all, which
//! is the documented degradation in that module rather than a wrong row set.
//!
//! # Why a policy row is a pair
//!
//! The flattening from `{role, members[]}` to one row per (role, member) pair
//! happens in the domain layer — see
//! [`nuvem_domain::gcp::resource::IamPolicyMember`] for why the wire shape is
//! not the shape a table should render, and
//! [`nuvem_domain::gcp::resource::IamMemberKind`] for the three member shapes
//! that are not `kind:id`.

use serde::Deserialize;

use nuvem_domain::gcp::configuration::GcloudContext;
use nuvem_domain::gcp::resource::{IamPolicyMember, ServiceAccount};

use crate::services::gcloud::cli::{CommandError, CommandRunner, GcloudCommand};
use crate::services::resources::list_filter::RESOURCE_LIST_LIMIT;

#[derive(Debug, thiserror::Error)]
pub enum IamParseError {
    #[error("invalid service accounts JSON: {0}")]
    ServiceAccounts(#[source] serde_json::Error),
    #[error("invalid IAM policy JSON: {0}")]
    Policy(#[source] serde_json::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum IamLoadError {
    #[error("{0}")]
    Command(#[from] CommandError),
    #[error("{0}")]
    Parse(#[from] IamParseError),
    /// `get-iam-policy` names its project positionally, so there is no call to
    /// make without one.
    #[error("no project is set, so this configuration has no IAM policy to read")]
    MissingProject,
}

pub struct GcloudIamService<R> {
    runner: R,
}

impl<R> GcloudIamService<R> {
    pub fn new(runner: R) -> Self {
        Self { runner }
    }

    pub fn runner(&self) -> &R {
        &self.runner
    }
}

impl<R: CommandRunner> GcloudIamService<R> {
    /// Lists a project's service accounts, capped at [`RESOURCE_LIST_LIMIT`]
    /// rows.
    ///
    /// `filter` is accepted and never pushed, for the reason in the module
    /// docs. It is still a parameter so this load has the same shape as every
    /// sibling service's, and so the day a pushable field exists there is no
    /// signature change to ripple.
    pub fn load_service_accounts(
        &self,
        context: &GcloudContext,
        _filter: Option<&str>,
    ) -> Result<Vec<ServiceAccount>, IamLoadError> {
        let mut args = base_args(context);
        args.extend([
            "iam".to_string(),
            "service-accounts".to_string(),
            "list".to_string(),
            "--format=json".to_string(),
            format!("--limit={RESOURCE_LIST_LIMIT}"),
        ]);
        let output = self.runner.run(GcloudCommand::new(args))?;
        Ok(parse_service_accounts_json(&output.stdout)?)
    }

    /// Reads the project's IAM policy and flattens it to one row per (role,
    /// member) pair.
    ///
    /// No `--limit`: the answer is a single policy object, and gcloud's limit
    /// counts resources rather than the bindings inside one — see the module
    /// docs.
    pub fn load_policy_members(
        &self,
        context: &GcloudContext,
    ) -> Result<Vec<IamPolicyMember>, IamLoadError> {
        let project = context
            .project
            .as_deref()
            .ok_or(IamLoadError::MissingProject)?;
        let mut args = base_args(context);
        args.extend([
            "projects".to_string(),
            "get-iam-policy".to_string(),
            project.to_string(),
            "--format=json".to_string(),
        ]);
        let output = self.runner.run(GcloudCommand::new(args))?;
        Ok(parse_policy_json(&output.stdout)?)
    }
}

/// Shared leading args: `--configuration <cfg> [--project <p>]`.
///
/// `get-iam-policy` also names the project positionally. The flag stays
/// alongside it: they agree by construction — both come from
/// `context.project` — and dropping the flag would make the two loads' argv
/// differ in a way that has nothing to do with what they ask for.
fn base_args(context: &GcloudContext) -> Vec<String> {
    let mut args = vec![
        "--configuration".to_string(),
        context.configuration.as_str().to_string(),
    ];
    if let Some(project) = context.project.as_deref() {
        args.extend(["--project".to_string(), project.to_string()]);
    }
    args
}

/// The wire shape of a service account, list format.
///
/// Every field defaults, so a row missing any of them still parses; only a
/// document that is not the expected JSON fails the load, which is the same
/// bargain every sibling service strikes.
///
/// `name`, `projectId`, `etag` and `oauth2ClientId` are read and discarded by
/// omission — see [`ServiceAccount`] for why none of them earns a field.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawServiceAccount {
    #[serde(default)]
    email: String,
    #[serde(default)]
    display_name: Option<String>,
    /// Absent means enabled. `disabled` is a proto3 boolean, and proto3 JSON
    /// omits `false`, so a missing key is the *value* `false` rather than an
    /// unknown — which is why this defaults instead of being an `Option`.
    #[serde(default)]
    disabled: bool,
    #[serde(default)]
    unique_id: String,
}

pub fn parse_service_accounts_json(stdout: &str) -> Result<Vec<ServiceAccount>, IamParseError> {
    let raw: Vec<RawServiceAccount> =
        serde_json::from_str(stdout).map_err(IamParseError::ServiceAccounts)?;
    Ok(raw.into_iter().map(service_account_from_raw).collect())
}

fn service_account_from_raw(raw: RawServiceAccount) -> ServiceAccount {
    ServiceAccount {
        email: raw.email,
        // An empty `displayName` is no display name. The API returns the key
        // with an empty string for an account created without one, and a
        // `Some("")` here would be a name the table paints as blank while
        // every `is_some()` check downstream believed there was one.
        display_name: raw.display_name.filter(|name| !name.is_empty()),
        disabled: raw.disabled,
        unique_id: raw.unique_id,
    }
}

/// The wire shape of a project IAM policy.
#[derive(Debug, Deserialize)]
struct RawPolicy {
    /// Absent when the project has no bindings at all, which gcloud renders by
    /// omitting the key rather than by an empty array.
    #[serde(default)]
    bindings: Vec<RawBinding>,
}

#[derive(Debug, Deserialize)]
struct RawBinding {
    #[serde(default)]
    role: String,
    #[serde(default)]
    members: Vec<String>,
    #[serde(default)]
    condition: Option<RawCondition>,
}

/// A binding's IAM condition. Only the title reaches a row — the `expression`
/// is CEL and the `description` is prose, and neither fits a table cell.
#[derive(Debug, Deserialize)]
struct RawCondition {
    #[serde(default)]
    title: Option<String>,
}

/// Flattens a policy to one row per (role, member) pair, in wire order.
///
/// A binding with an empty `members` array contributes no rows. That is the
/// honest answer: the pair is the unit this table renders, and a role nobody
/// holds is not one — a placeholder row would put a grant on screen that the
/// policy does not contain.
pub fn parse_policy_json(stdout: &str) -> Result<Vec<IamPolicyMember>, IamParseError> {
    let raw: RawPolicy = serde_json::from_str(stdout).map_err(IamParseError::Policy)?;
    Ok(raw
        .bindings
        .into_iter()
        .flat_map(|binding| {
            let title = binding
                .condition
                .and_then(|condition| condition.title)
                .unwrap_or_default();
            let role = binding.role;
            binding
                .members
                .into_iter()
                .map(move |member| IamPolicyMember::from_wire(&role, &member, Some(title.as_str())))
        })
        .collect())
}
