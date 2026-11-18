use std::path::PathBuf;

use nuvem::Config;
use nuvem::app::effect::InteractiveCommand;
use nuvem::app::{Action, AppState, Effect};
use nuvem::services::gcloud::cli::{CommandError, CommandOutput, CommandRunner, GcloudCommand};
use nuvem::services::runtime::session::RuntimeSession;
use nuvem_domain::gcp::configuration::{
    GcloudConfiguration, GcloudConfigurationName, GcloudContext,
};
use nuvem_domain::gcp::resource::{ComputeInstance, ResourceStatus};

/// A runner that must never actually run: routing an interactive launch never
/// touches the gcloud command runner (the launch is stashed for the run loop,
/// and spec resolution reads only configuration).
struct UnusedRunner;

impl CommandRunner for UnusedRunner {
    fn run(&self, _command: GcloudCommand) -> Result<CommandOutput, CommandError> {
        panic!("interactive routing must not invoke the gcloud command runner");
    }
}

fn config_with_binary(binary: &str) -> Config {
    Config {
        gcloud_binary: PathBuf::from(binary),
        ..Config::default()
    }
}

fn context() -> GcloudContext {
    GcloudConfiguration {
        name: GcloudConfigurationName::new("dev"),
        is_active: true,
        account: Some("dev@example.com".to_string()),
        project: Some("dev-project".to_string()),
        region: None,
        zone: None,
    }
    .context()
}

fn instance() -> ComputeInstance {
    ComputeInstance {
        id: "1001".to_string(),
        name: "api-dev-1".to_string(),
        zone: "europe-west1-b".to_string(),
        status: ResourceStatus::Running,
        machine_type: "e2-medium".to_string(),
        internal_ip: Some("10.0.0.4".to_string()),
        external_ip: Some("34.10.10.10".to_string()),
    }
}

fn session_with_binary(binary: &str) -> RuntimeSession {
    RuntimeSession::new_with_config_and_runner_for_test(
        config_with_binary(binary),
        AppState::new(),
        Box::new(UnusedRunner),
    )
}

#[tokio::test]
async fn launch_effect_is_intercepted_into_pending_slot() {
    let mut session = session_with_binary("/custom/gcloud");
    let command = InteractiveCommand::GceSsh {
        context: context(),
        instance: instance(),
    };

    session
        .dispatch_action(Action::Effect(Effect::LaunchInteractiveProcess {
            command: command.clone(),
        }))
        .await;

    assert_eq!(
        session.take_pending_interactive_command(),
        Some(command),
        "the launch effect must be stashed, not dispatched to gcloud"
    );
    assert_eq!(
        session.take_pending_interactive_command(),
        None,
        "the pending slot must be drained by the first take"
    );
}

#[test]
fn resolve_interactive_spec_uses_configured_binary() {
    let session = session_with_binary("/custom/gcloud");
    let command = InteractiveCommand::GceSsh {
        context: context(),
        instance: instance(),
    };

    let spec = session.resolve_interactive_spec(&command);

    assert_eq!(spec.program, "/custom/gcloud");
    assert_eq!(
        spec.args,
        vec![
            "--configuration".to_string(),
            "dev".to_string(),
            "compute".to_string(),
            "ssh".to_string(),
            "api-dev-1".to_string(),
            "--zone".to_string(),
            "europe-west1-b".to_string(),
        ]
    );
    assert_eq!(spec.description, "SSH to api-dev-1");
}
