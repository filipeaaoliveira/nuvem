//! Secret Manager, metadata only.
//!
//! # The subcommands this module may issue
//!
//! `secrets list`, `secrets describe`, `secrets versions list`. Those three,
//! and no others.
//!
//! In particular this module never issues the version subcommand that returns
//! payload material, and no other module does either. That is the whole point
//! of the service: nuvem browses Secret Manager metadata and a payload can
//! never render, because the call that would fetch one does not exist anywhere
//! in this binary — not because a render layer redacts it afterwards.
//!
//! The payload subcommand and its API method are deliberately not spelled out
//! anywhere in `src/` or `crates/`, including in prose like this one. The scan
//! that enforces the guarantee is a plain whole-tree token search with no
//! comment parsing and therefore no way to be argued around, and that bluntness
//! is worth more than the convenience of naming the thing here.
//!
//! This comment does not enforce that. `tests/secrets_no_payload_access_test.rs`
//! does, from two directions: it enumerates every argv this module can build and
//! asserts none reaches the payload subcommand, and it scans the whole source
//! tree for the token. A guarantee that lives only in a comment is not a
//! guarantee.

use serde::Deserialize;

use nuvem_domain::gcp::configuration::GcloudContext;
use nuvem_domain::gcp::filter::label_filter_field;
use nuvem_domain::gcp::resource::{
    ComputeKeyValue, ResourceServiceKind, Secret, SecretDetail, SecretVersion, SecretVersionState,
};

use crate::services::gcloud::cli::{CommandError, CommandRunner, GcloudCommand};
use crate::services::resources::list_filter::{RESOURCE_LIST_LIMIT, pushdown_expression};

/// Verbatim-superset raw fields eligible for the regex pushdown in
/// [`GcloudSecretsService::load_secrets`] — deliberately none.
///
/// A secret's searchable display values are its name, its replication policy,
/// its create time, and its labels rendered as `key=value`. Replication is
/// derived (the raw JSON is a nested `replication.automatic` /
/// `replication.userManaged` object, not the string on screen) and the
/// `key=value` label rendering exists in no raw field either, so a term
/// matching one of those client-side would match nothing pushed and the row
/// would vanish. Rather than enumerate every replication rendering as an
/// opaque label to gate them out, this service pushes no regex clause at all
/// and narrows regex mode entirely client-side.
const FILTER_FIELDS: &[&str] = &[];

/// Empty for the same reason [`FILTER_FIELDS`] is: with nothing pushed there
/// is nothing for the gate to protect.
const OPAQUE_LABELS: &[&str] = &[];

#[derive(Debug, thiserror::Error)]
pub enum SecretsParseError {
    #[error("invalid Secret Manager JSON: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum SecretsLoadError {
    #[error("{0}")]
    Command(#[from] CommandError),
    #[error("{0}")]
    Parse(#[from] SecretsParseError),
}

pub struct GcloudSecretsService<R> {
    runner: R,
}

impl<R> GcloudSecretsService<R> {
    pub fn new(runner: R) -> Self {
        Self { runner }
    }

    pub fn runner(&self) -> &R {
        &self.runner
    }
}

impl<R: CommandRunner> GcloudSecretsService<R> {
    /// Lists secrets, capped at [`RESOURCE_LIST_LIMIT`] rows and, in label
    /// mode, narrowed with a pushed-down `--filter` on `labels`.
    ///
    /// Only label mode reaches the server. `FILTER_FIELDS` is empty, so the
    /// regex arm of [`pushdown_expression`] always returns `None` and the
    /// client-side filter does the narrowing: a secret's list row carries no
    /// raw JSON field this module has audited as a verbatim superset of a
    /// display value, and the never-drop-a-row invariant needs one. Label mode
    /// has no such fallback — `CompiledFilter::Label` keeps every row — so
    /// pushing it is what makes `-l k=v` narrow at all.
    pub fn load_secrets(
        &self,
        context: &GcloudContext,
        filter: Option<&str>,
    ) -> Result<Vec<Secret>, SecretsLoadError> {
        let mut args = base_args(context);
        args.extend([
            "secrets".to_string(),
            "list".to_string(),
            "--format=json".to_string(),
            format!("--limit={RESOURCE_LIST_LIMIT}"),
        ]);
        if let Some(expression) = filter.and_then(|applied| {
            pushdown_expression(
                applied,
                FILTER_FIELDS,
                OPAQUE_LABELS,
                label_filter_field(ResourceServiceKind::SecretManagerSecrets),
            )
        }) {
            args.push(format!("--filter={expression}"));
        }
        let output = self.runner.run(GcloudCommand::new(args))?;
        Ok(parse_secrets_json(&output.stdout)?)
    }

    pub fn load_secret_detail(
        &self,
        context: &GcloudContext,
        secret: &str,
    ) -> Result<SecretDetail, SecretsLoadError> {
        let mut args = base_args(context);
        args.extend([
            "secrets".to_string(),
            "describe".to_string(),
            secret.to_string(),
            "--format=json".to_string(),
        ]);
        let output = self.runner.run(GcloudCommand::new(args))?;
        Ok(parse_secret_detail_json(&output.stdout)?)
    }

    /// Lists a secret's versions. Metadata only: name, state, create time. Its
    /// sibling subcommand — the one that returns payload material — is not
    /// issued here or anywhere else in this binary.
    pub fn load_secret_versions(
        &self,
        context: &GcloudContext,
        secret: &str,
    ) -> Result<Vec<SecretVersion>, SecretsLoadError> {
        let mut args = base_args(context);
        args.extend([
            "secrets".to_string(),
            "versions".to_string(),
            "list".to_string(),
            secret.to_string(),
            "--format=json".to_string(),
        ]);
        let output = self.runner.run(GcloudCommand::new(args))?;
        Ok(parse_secret_versions_json(&output.stdout)?)
    }
}

/// Shared leading args: `--configuration <cfg> [--project <p>]`.
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

/// The wire shape of a secret, shared by `list` and `describe`.
///
/// Every field is metadata. There is no payload field to leave out, because the
/// API responses these parse never carry one: `list` and `describe` return the
/// secret resource, not its versions' material.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawSecret {
    #[serde(default)]
    name: String,
    #[serde(default)]
    create_time: Option<String>,
    #[serde(default)]
    labels: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    annotations: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    replication: Option<RawReplication>,
    #[serde(default)]
    rotation: Option<RawRotation>,
    #[serde(default)]
    expire_time: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawReplication {
    #[serde(default)]
    automatic: Option<serde_json::Value>,
    #[serde(default)]
    user_managed: Option<RawUserManaged>,
}

#[derive(Debug, Deserialize)]
struct RawUserManaged {
    #[serde(default)]
    replicas: Vec<RawReplica>,
}

#[derive(Debug, Deserialize)]
struct RawReplica {
    #[serde(default)]
    location: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawRotation {
    #[serde(default)]
    next_rotation_time: Option<String>,
    #[serde(default)]
    rotation_period: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawSecretVersion {
    #[serde(default)]
    name: String,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    create_time: Option<String>,
}

/// The trailing segment of a fully-qualified resource path.
///
/// The API returns `projects/p/secrets/s` and
/// `projects/p/secrets/s/versions/3`; a table cell wants `s` and `3`. A name
/// with no slash is already short and passes through unchanged.
pub fn short_id(resource_path: &str) -> String {
    resource_path
        .rsplit('/')
        .next()
        .unwrap_or(resource_path)
        .to_string()
}

/// Sorted key/value pairs. Sorted rather than in map order so a reload does not
/// reshuffle the detail pane under the operator's cursor.
pub fn key_values(map: std::collections::BTreeMap<String, String>) -> Vec<ComputeKeyValue> {
    map.into_iter()
        .map(|(key, value)| ComputeKeyValue { key, value })
        .collect()
}

/// Renders a replication policy into the one line a table cell shows.
///
/// A user-managed policy names its replica locations: "user-managed" alone says
/// only that someone chose, and *where* the material is replicated is the fact
/// an operator is checking when they look at this column.
fn replication_label(replication: Option<RawReplication>) -> String {
    let Some(replication) = replication else {
        return replication_label_from(false, None);
    };
    if replication.automatic.is_some() {
        return replication_label_from(true, None);
    }
    replication_label_from(
        false,
        replication.user_managed.map(|user_managed| {
            user_managed
                .replicas
                .into_iter()
                .filter_map(|replica| replica.location)
                .collect::<Vec<_>>()
        }),
    )
}

/// The label itself, over primitives, so the CLI's JSON shape and the native
/// client's oneof both reach the same words. The wording is a cell an operator
/// reads; two copies of it is two things to keep in step.
pub fn replication_label_from(
    automatic: bool,
    user_managed_locations: Option<Vec<String>>,
) -> String {
    if automatic {
        return "Automatic".to_string();
    }
    match user_managed_locations {
        Some(locations) if locations.is_empty() => "User-managed".to_string(),
        Some(locations) => format!("User-managed ({})", locations.join(", ")),
        None => "Unknown".to_string(),
    }
}

/// Renders a rotation policy, or `None` when the secret has none. A policy with
/// neither a period nor a next time carries no information and reads as absent.
fn rotation_label(rotation: Option<RawRotation>) -> Option<String> {
    let rotation = rotation?;
    rotation_label_from(rotation.rotation_period, rotation.next_rotation_time)
}

/// The rotation cell, from the two values both backends carry.
///
/// Shared so the wording cannot drift between the paths.
pub fn rotation_label_from(period: Option<String>, next: Option<String>) -> Option<String> {
    match (period, next) {
        (Some(period), Some(next)) => Some(format!("every {period}, next {next}")),
        (Some(period), None) => Some(format!("every {period}")),
        (None, Some(next)) => Some(format!("next {next}")),
        (None, None) => None,
    }
}

fn secret_from_raw(raw: RawSecret) -> Secret {
    Secret {
        name: short_id(&raw.name),
        create_time: raw.create_time,
        labels: key_values(raw.labels),
        replication: replication_label(raw.replication),
    }
}

pub fn parse_secrets_json(stdout: &str) -> Result<Vec<Secret>, SecretsParseError> {
    let raw: Vec<RawSecret> = serde_json::from_str(stdout)?;
    Ok(raw.into_iter().map(secret_from_raw).collect())
}

pub fn parse_secret_detail_json(stdout: &str) -> Result<SecretDetail, SecretsParseError> {
    let raw: RawSecret = serde_json::from_str(stdout)?;
    let raw_json = pretty_json(stdout);
    Ok(SecretDetail {
        name: short_id(&raw.name),
        create_time: raw.create_time,
        labels: key_values(raw.labels),
        annotations: key_values(raw.annotations),
        replication: replication_label(raw.replication),
        rotation: rotation_label(raw.rotation),
        expire_time: raw.expire_time,
        // Filled by the separate, on-demand versions fetch. `describe` does not
        // return versions, and this is also where the version count comes from.
        versions: Vec::new(),
        raw_json,
    })
}

pub fn parse_secret_versions_json(stdout: &str) -> Result<Vec<SecretVersion>, SecretsParseError> {
    let raw: Vec<RawSecretVersion> = serde_json::from_str(stdout)?;
    Ok(raw
        .into_iter()
        .map(|version| SecretVersion {
            name: short_id(&version.name),
            state: version_state(version.state.as_deref()),
            create_time: version.create_time,
        })
        .collect())
}

/// Maps the API's state string onto [`SecretVersionState`], carrying anything
/// unrecognised through verbatim rather than collapsing it into a known state —
/// a version whose state nuvem does not understand must not read as `Enabled`.
pub fn version_state(raw: Option<&str>) -> SecretVersionState {
    match raw {
        Some("ENABLED") => SecretVersionState::Enabled,
        Some("DISABLED") => SecretVersionState::Disabled,
        Some("DESTROYED") => SecretVersionState::Destroyed,
        Some(other) => SecretVersionState::Unknown(other.to_string()),
        None => SecretVersionState::Unknown(String::new()),
    }
}

/// Pretty-prints the describe response for the JSON tab, falling back to the
/// raw text when it does not parse.
///
/// Unlike Cloud SQL's and Cloud Storage's equivalents this runs no redaction
/// pass, and does not need one: the secret resource has no payload field, so
/// there is nothing in this JSON to redact. The guarantee is upstream — the
/// call that returns payloads is never made.
fn pretty_json(stdout: &str) -> String {
    serde_json::from_str::<serde_json::Value>(stdout)
        .and_then(|value| serde_json::to_string_pretty(&value))
        .unwrap_or_else(|_| stdout.to_string())
}
