//! `RuntimeSession::effect_tasks_in_flight` is the settled-or-not signal the
//! render harnesses read. It counts sender clones held by live effect tasks,
//! so it must be zero on a fresh session, non-zero the moment a fetch has been
//! spawned, and zero again once the channel has been drained.

use nuvem::Config;
use nuvem::app::{Action, AppState, ResourcesAction, WorkspaceAction};
use nuvem::services::demo::DemoCommandRunner;
use nuvem::services::runtime::session::RuntimeSession;
use nuvem_domain::gcp::resource::ResourceServiceKind;

fn demo_session() -> RuntimeSession {
    let config = Config {
        demo: true,
        ..Config::default()
    };
    let mut app = AppState::new();
    nuvem::services::runtime::apply_startup_config(&mut app, &config);
    RuntimeSession::new_with_config_and_runner_for_test(
        config,
        app,
        Box::new(DemoCommandRunner::new()),
    )
}

#[test]
fn a_fresh_session_has_nothing_in_flight() {
    assert_eq!(demo_session().effect_tasks_in_flight(), 0);
}

#[test]
fn a_spawned_fetch_is_in_flight_until_its_result_is_drained() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    runtime.block_on(async {
        let mut session = demo_session();
        session.initialize().await;
        session
            .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
            .await;
        session
            .dispatch_action(Action::Resources(ResourcesAction::SelectService(
                ResourceServiceKind::ComputeInstances,
            )))
            .await;
        assert!(
            session.effect_tasks_in_flight() > 0,
            "selecting a service spawns its fetch, which must show as in flight"
        );

        // Drain to a settled state the same way the harnesses do.
        for _ in 0..4096 {
            let quiet = !matches!(
                tokio::time::timeout(
                    std::time::Duration::from_millis(5),
                    session.process_next_effect_result(),
                )
                .await,
                Ok(true)
            );
            if quiet && session.effect_tasks_in_flight() == 0 {
                break;
            }
        }
        assert_eq!(
            session.effect_tasks_in_flight(),
            0,
            "once every result is drained no task may still hold a sender"
        );
    });
}
