use nuvem::app::{Action, AppState, EffectResult};
use nuvem::services::runtime::session::RuntimeSession;
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};

#[tokio::test]
async fn session_applies_loaded_configurations() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![GcloudConfiguration {
                name: GcloudConfigurationName::new("dev"),
                is_active: true,
                account: Some("dev@example.com".to_string()),
                project: Some("dev-project".to_string()),
                region: None,
                zone: None,
            }],
        })
        .await;

    assert_eq!(
        session
            .app()
            .root
            .current_context()
            .unwrap()
            .configuration
            .as_str(),
        "dev"
    );

    assert!(!session.dispatch_action(Action::QuitRequested).await);
}
