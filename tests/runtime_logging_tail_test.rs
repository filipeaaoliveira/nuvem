use nuvem::app::AppState;
use nuvem::app::reducer::root::reduce;
use nuvem::app::state::logging::TailState;
use nuvem::app::state::root::Workspace;
use nuvem::app::{Action, EffectResult, WorkspaceAction};
use nuvem::services::runtime::tail::{TailDecision, should_tail};
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use nuvem_domain::gcp::logging::LogCursor;

fn config(name: &str, active: bool, project: &str) -> GcloudConfiguration {
    GcloudConfiguration {
        name: GcloudConfigurationName::new(name),
        is_active: active,
        account: None,
        project: Some(project.to_string()),
        region: None,
        zone: None,
    }
}

/// Seeds one active configuration so `current_context()` is `Some`, mirroring
/// the pattern in `reducer_logging_test.rs`.
fn app_with_active_config() -> AppState {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        }),
    );
    app
}

fn cursor() -> LogCursor {
    LogCursor {
        timestamp: "2026-07-19T00:00:00Z".to_string(),
        insert_ids: vec!["abc".to_string()],
    }
}

#[test]
fn tails_when_running_with_cursor_no_backoff_no_pending() {
    let mut app = app_with_active_config();
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Logging));
    app.root.logging.cursor = Some(cursor());
    app.root.logging.backoff_ticks = 0;
    assert_eq!(should_tail(&app.root, 0), TailDecision::Tail);
}

#[test]
fn loads_first_page_when_running_without_cursor() {
    let mut app = app_with_active_config();
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Logging));
    app.root.logging.cursor = None;
    app.root.logging.backoff_ticks = 0;
    assert_eq!(should_tail(&app.root, 0), TailDecision::LoadFirstPage);
}

#[test]
fn skips_when_tail_paused() {
    let mut app = app_with_active_config();
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Logging));
    app.root.logging.cursor = Some(cursor());
    app.root.logging.tail = TailState::Paused;
    assert_eq!(should_tail(&app.root, 0), TailDecision::Skip);
}

#[test]
fn skips_when_pending_refresh_in_flight() {
    let mut app = app_with_active_config();
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Logging));
    app.root.logging.cursor = Some(cursor());
    assert_eq!(should_tail(&app.root, 1), TailDecision::Skip);
}

#[test]
fn skips_while_backoff_pending() {
    let mut app = app_with_active_config();
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Logging));
    app.root.logging.cursor = Some(cursor());
    app.root.logging.backoff_ticks = 2;
    assert_eq!(should_tail(&app.root, 0), TailDecision::Skip);
}

#[test]
fn skips_outside_logging_workspace() {
    let app = app_with_active_config();
    assert_ne!(app.root.workspace, Workspace::Logging);
    assert_eq!(should_tail(&app.root, 0), TailDecision::Skip);
}

#[test]
fn skips_without_active_context() {
    let mut app = AppState::new();
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Logging));
    app.root.logging.cursor = Some(cursor());
    assert!(app.root.current_context().is_none());
    assert_eq!(should_tail(&app.root, 0), TailDecision::Skip);
}

/// Documents the skipped-tick semantics: a backoff of N ticks drains to zero
/// over exactly N calls to `decrement_backoff`, each returning `false` until
/// the final call (which clears it and returns `true`).
#[test]
fn decrement_backoff_drains_over_n_calls() {
    let mut app = app_with_active_config();
    app.root.logging.backoff_ticks = 3;
    assert!(!app.root.logging.decrement_backoff());
    assert_eq!(app.root.logging.backoff_ticks, 2);
    assert!(!app.root.logging.decrement_backoff());
    assert_eq!(app.root.logging.backoff_ticks, 1);
    assert!(app.root.logging.decrement_backoff());
    assert_eq!(app.root.logging.backoff_ticks, 0);
    // Draining an already-clear backoff stays clear.
    assert!(app.root.logging.decrement_backoff());
    assert_eq!(app.root.logging.backoff_ticks, 0);
}

/// Regression test for the composed sequencing `poll_active_tail` must use:
/// compute the decision on the CURRENT (pre-decrement) backoff value, then
/// drain exactly one tick. A backoff of N must produce N fully-skipped ticks
/// before tailing resumes on tick N+1 — i.e. backoff=2 skips twice, then
/// tails on the third tick.
#[test]
fn composed_decision_then_drain_skips_n_ticks_for_backoff_n() {
    let mut app = app_with_active_config();
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Logging));
    app.root.logging.cursor = Some(cursor());
    app.root.logging.backoff_ticks = 2;

    let mut decisions = Vec::new();
    for _ in 0..3 {
        let decision = should_tail(&app.root, 0);
        app.root.logging.decrement_backoff();
        decisions.push(decision);
    }

    assert_eq!(
        decisions,
        vec![TailDecision::Skip, TailDecision::Skip, TailDecision::Tail]
    );
}

/// The tail poll's first-page load is the one fetch no reducer marked, so the
/// poll marks it: a viewer that was showing "Not loaded" (nothing was ever
/// fetched, because the context did not resolve when the workspace was entered)
/// must not keep saying so once that context resolves and the poll starts a
/// load.
#[tokio::test]
async fn tail_poll_first_page_marks_the_buffer_loading() {
    use nuvem::app::state::logging::LoggingStatus;
    use nuvem::services::demo::DemoCommandRunner;
    use nuvem::services::runtime::session::RuntimeSession;

    // Entered Logging with no context: nothing was dispatched, nothing loaded.
    let mut app = AppState::new();
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Logging));
    assert!(app.root.current_context().is_none());
    assert_eq!(app.root.logging.status, LoggingStatus::Initial);

    let mut session =
        RuntimeSession::new_with_runner_for_test(app, Box::new(DemoCommandRunner::new()));
    // ...and now the context resolves, without the workspace changing.
    session
        .dispatch_action(Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        }))
        .await;
    assert!(session.app().root.current_context().is_some());

    session.poll_active_tail().await;
    assert_eq!(
        session.app().root.logging.status,
        LoggingStatus::Loading,
        "the poll started a first page, so the buffer must say a load is running"
    );
}
