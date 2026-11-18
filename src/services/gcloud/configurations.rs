use serde::Deserialize;

use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};

use super::cli::{CommandError, CommandRunner, GcloudCommand};

#[derive(Debug, thiserror::Error)]
pub enum GcloudConfigurationParseError {
    #[error("invalid gcloud configuration JSON: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum GcloudConfigurationError {
    #[error(transparent)]
    Command(#[from] CommandError),
    #[error("gcloud configuration {name} was created, but setup failed: {source}")]
    CreatedConfigurationSetupFailed {
        name: String,
        #[source]
        source: CommandError,
    },
    /// The one boxed source in this enum, and it is boxed for its size rather
    /// than for its meaning: this variant already carries 72 bytes of
    /// `applied`, and when `CommandError::Failed` gained the child's
    /// stdout beside its stderr the pair crossed `clippy::result_large_err`'s
    /// 128-byte line — so every function returning this enum paid the width of
    /// a diagnostic that only one of its four variants holds. Boxing here
    /// rather than shrinking `CommandError` keeps the cost where it is
    /// incurred; `Failed` is constructed and matched in ~60 places and reads
    /// as two plain strings in all of them.
    #[error(
        "gcloud configuration context update failed after native state may have changed: {source}"
    )]
    ContextUpdatePartiallyApplied {
        applied: GcloudContextAppliedProperties,
        #[source]
        source: Box<CommandError>,
    },
    #[error(transparent)]
    Parse(#[from] GcloudConfigurationParseError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GcloudConfigurationDraft {
    pub name: String,
    pub account: Option<String>,
    pub project: Option<String>,
    pub region: Option<String>,
    pub zone: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GcloudContextEditDraft {
    pub configuration: GcloudConfigurationName,
    pub project: Option<String>,
    pub region: Option<String>,
    pub zone: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GcloudContextAppliedProperties {
    pub project: Option<Option<String>>,
    pub region: Option<Option<String>>,
    pub zone: Option<Option<String>>,
}

pub struct GcloudConfigurationService<R> {
    runner: R,
}

impl<R> GcloudConfigurationService<R> {
    pub fn new(runner: R) -> Self {
        Self { runner }
    }

    pub fn runner(&self) -> &R {
        &self.runner
    }
}

impl<R: CommandRunner> GcloudConfigurationService<R> {
    pub fn load_configurations(
        &self,
    ) -> Result<Vec<GcloudConfiguration>, GcloudConfigurationError> {
        let output = self.runner.run(GcloudCommand::new([
            "config",
            "configurations",
            "list",
            "--format=json",
        ]))?;
        Ok(parse_configurations_json(&output.stdout)?)
    }

    pub fn activate_configuration(
        &self,
        name: &GcloudConfigurationName,
    ) -> Result<(), GcloudConfigurationError> {
        self.runner.run(GcloudCommand::new([
            "config",
            "configurations",
            "activate",
            name.as_str(),
        ]))?;
        Ok(())
    }

    pub fn create_configuration(
        &self,
        draft: GcloudConfigurationDraft,
    ) -> Result<(), GcloudConfigurationError> {
        let name = draft.name;
        self.runner.run(GcloudCommand::new([
            "config",
            "configurations",
            "create",
            name.as_str(),
        ]))?;
        if let Some(account) = draft.account {
            self.runner
                .run(GcloudCommand::new([
                    "--configuration",
                    name.as_str(),
                    "config",
                    "set",
                    "account",
                    account.as_str(),
                ]))
                .map_err(
                    |source| GcloudConfigurationError::CreatedConfigurationSetupFailed {
                        name: name.clone(),
                        source,
                    },
                )?;
        }
        if let Some(project) = draft.project {
            self.runner
                .run(GcloudCommand::new([
                    "--configuration",
                    name.as_str(),
                    "config",
                    "set",
                    "project",
                    project.as_str(),
                ]))
                .map_err(
                    |source| GcloudConfigurationError::CreatedConfigurationSetupFailed {
                        name: name.clone(),
                        source,
                    },
                )?;
        }
        if let Some(region) = draft.region {
            self.runner
                .run(GcloudCommand::new([
                    "--configuration",
                    name.as_str(),
                    "config",
                    "set",
                    "compute/region",
                    region.as_str(),
                ]))
                .map_err(
                    |source| GcloudConfigurationError::CreatedConfigurationSetupFailed {
                        name: name.clone(),
                        source,
                    },
                )?;
        }
        if let Some(zone) = draft.zone {
            self.runner
                .run(GcloudCommand::new([
                    "--configuration",
                    name.as_str(),
                    "config",
                    "set",
                    "compute/zone",
                    zone.as_str(),
                ]))
                .map_err(
                    |source| GcloudConfigurationError::CreatedConfigurationSetupFailed {
                        name: name.clone(),
                        source,
                    },
                )?;
        }
        Ok(())
    }

    pub fn update_context(
        &self,
        draft: GcloudContextEditDraft,
    ) -> Result<GcloudContextAppliedProperties, GcloudConfigurationError> {
        let mut applied = GcloudContextAppliedProperties::default();
        self.runner.run(property_command(
            &draft.configuration,
            "project",
            draft.project.as_deref(),
        ))?;
        applied.project = Some(draft.project.clone());
        self.runner
            .run(property_command(
                &draft.configuration,
                "compute/region",
                draft.region.as_deref(),
            ))
            .map_err(
                |source| GcloudConfigurationError::ContextUpdatePartiallyApplied {
                    applied: applied.clone(),
                    source: Box::new(source),
                },
            )?;
        applied.region = Some(draft.region.clone());
        self.runner
            .run(property_command(
                &draft.configuration,
                "compute/zone",
                draft.zone.as_deref(),
            ))
            .map_err(
                |source| GcloudConfigurationError::ContextUpdatePartiallyApplied {
                    applied: applied.clone(),
                    source: Box::new(source),
                },
            )?;
        applied.zone = Some(draft.zone.clone());
        Ok(applied)
    }
}

fn property_command(
    configuration: &GcloudConfigurationName,
    property: &'static str,
    value: Option<&str>,
) -> GcloudCommand {
    let mut args = vec![
        "--configuration".to_string(),
        configuration.as_str().to_string(),
        "config".to_string(),
    ];
    match value {
        Some(value) => {
            args.push("set".to_string());
            args.push(property.to_string());
            args.push(value.to_string());
        }
        None => {
            args.push("unset".to_string());
            args.push(property.to_string());
        }
    }
    GcloudCommand::new(args)
}

#[derive(Debug, Deserialize)]
struct RawConfiguration {
    name: String,
    #[serde(default)]
    is_active: bool,
    account: Option<String>,
    project: Option<String>,
    #[serde(default)]
    properties: RawProperties,
}

#[derive(Debug, Default, Deserialize)]
struct RawProperties {
    #[serde(default)]
    core: RawCore,
    #[serde(default)]
    compute: RawCompute,
}

#[derive(Debug, Default, Deserialize)]
struct RawCore {
    account: Option<String>,
    project: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct RawCompute {
    region: Option<String>,
    zone: Option<String>,
}

pub fn parse_configurations_json(
    json: &str,
) -> Result<Vec<GcloudConfiguration>, GcloudConfigurationParseError> {
    let raw: Vec<RawConfiguration> = serde_json::from_str(json)?;
    Ok(raw
        .into_iter()
        .map(|item| GcloudConfiguration {
            name: GcloudConfigurationName::new(item.name),
            is_active: item.is_active,
            account: item.properties.core.account.or(item.account),
            project: item.properties.core.project.or(item.project),
            region: item.properties.compute.region,
            zone: item.properties.compute.zone,
        })
        .collect())
}
