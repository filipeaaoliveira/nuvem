use serde::Deserialize;
use tokio_util::sync::CancellationToken;

use nuvem_domain::gcp::configuration::GcloudConfigurationName;
use nuvem_domain::gcp::location::{ComputeRegion, ComputeZone};
use nuvem_domain::gcp::project::GcloudProject;

use super::cli::{CommandError, CommandRunner, GcloudCommand};

#[derive(Debug, thiserror::Error)]
pub enum GcloudDiscoveryError {
    #[error("gcloud discovery command failed: {0}")]
    Command(#[from] CommandError),
    #[error("invalid gcloud discovery JSON: {0}")]
    Parse(#[from] serde_json::Error),
}

pub struct GcloudDiscoveryService<R> {
    runner: R,
}

impl<R> GcloudDiscoveryService<R> {
    pub fn new(runner: R) -> Self {
        Self { runner }
    }

    pub fn runner(&self) -> &R {
        &self.runner
    }
}

impl<R: CommandRunner> GcloudDiscoveryService<R> {
    pub fn load_projects(
        &self,
        configuration: &GcloudConfigurationName,
    ) -> Result<Vec<GcloudProject>, GcloudDiscoveryError> {
        let output = self.runner.run(GcloudCommand::new([
            "--configuration",
            configuration.as_str(),
            "projects",
            "list",
            "--format=json",
        ]))?;
        parse_projects_json(&output.stdout)
    }

    pub fn load_projects_with_cancellation(
        &self,
        configuration: &GcloudConfigurationName,
        cancellation: &CancellationToken,
    ) -> Result<Vec<GcloudProject>, GcloudDiscoveryError> {
        let output = self.runner.run_with_cancellation(
            GcloudCommand::new([
                "--configuration",
                configuration.as_str(),
                "projects",
                "list",
                "--format=json",
            ]),
            cancellation,
        )?;
        parse_projects_json(&output.stdout)
    }

    pub fn load_regions(
        &self,
        configuration: &GcloudConfigurationName,
    ) -> Result<Vec<ComputeRegion>, GcloudDiscoveryError> {
        let output = self.runner.run(GcloudCommand::new([
            "--configuration",
            configuration.as_str(),
            "compute",
            "regions",
            "list",
            "--format=json",
        ]))?;
        parse_regions_json(&output.stdout)
    }

    pub fn load_regions_with_cancellation(
        &self,
        configuration: &GcloudConfigurationName,
        cancellation: &CancellationToken,
    ) -> Result<Vec<ComputeRegion>, GcloudDiscoveryError> {
        let output = self.runner.run_with_cancellation(
            GcloudCommand::new([
                "--configuration",
                configuration.as_str(),
                "compute",
                "regions",
                "list",
                "--format=json",
            ]),
            cancellation,
        )?;
        parse_regions_json(&output.stdout)
    }

    pub fn load_zones(
        &self,
        configuration: &GcloudConfigurationName,
    ) -> Result<Vec<ComputeZone>, GcloudDiscoveryError> {
        let output = self.runner.run(GcloudCommand::new([
            "--configuration",
            configuration.as_str(),
            "compute",
            "zones",
            "list",
            "--format=json",
        ]))?;
        parse_zones_json(&output.stdout)
    }

    pub fn load_zones_with_cancellation(
        &self,
        configuration: &GcloudConfigurationName,
        cancellation: &CancellationToken,
    ) -> Result<Vec<ComputeZone>, GcloudDiscoveryError> {
        let output = self.runner.run_with_cancellation(
            GcloudCommand::new([
                "--configuration",
                configuration.as_str(),
                "compute",
                "zones",
                "list",
                "--format=json",
            ]),
            cancellation,
        )?;
        parse_zones_json(&output.stdout)
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawProject {
    project_id: String,
    name: String,
    #[serde(default)]
    lifecycle_state: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawRegion {
    name: String,
    #[serde(default)]
    status: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawZone {
    name: String,
    #[serde(default)]
    region: Option<String>,
    #[serde(default)]
    status: Option<String>,
}

pub fn parse_projects_json(json: &str) -> Result<Vec<GcloudProject>, GcloudDiscoveryError> {
    let raw: Vec<RawProject> = serde_json::from_str(json)?;
    Ok(raw
        .into_iter()
        .map(|item| GcloudProject {
            project_id: item.project_id,
            name: item.name,
            lifecycle_state: item.lifecycle_state,
        })
        .collect())
}

pub fn parse_regions_json(json: &str) -> Result<Vec<ComputeRegion>, GcloudDiscoveryError> {
    let raw: Vec<RawRegion> = serde_json::from_str(json)?;
    Ok(raw
        .into_iter()
        .map(|item| ComputeRegion {
            name: item.name,
            status: item.status,
        })
        .collect())
}

pub fn parse_zones_json(json: &str) -> Result<Vec<ComputeZone>, GcloudDiscoveryError> {
    let raw: Vec<RawZone> = serde_json::from_str(json)?;
    Ok(raw
        .into_iter()
        .map(|item| ComputeZone {
            name: item.name,
            region: item.region.as_deref().map(tail_segment),
            status: item.status,
        })
        .collect())
}

pub(crate) fn tail_segment(value: &str) -> String {
    value.rsplit('/').next().unwrap_or(value).to_string()
}
