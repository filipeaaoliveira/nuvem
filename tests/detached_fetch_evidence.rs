//! Evidence generator — both halves of the detached-fetch fix,
//! rendered as the user sees them.
//!
//! Everything here runs offline through `DemoCommandRunner` at the
//! `CommandRunner` seam, so the frames below are the real `nuvem::ui::render`
//! output over the real reducer and the real `RuntimeSession` scheduling — no
//! gcloud binary, no network, no fake UI.
//!
//! Half (i): a list fetch the runtime *dropped* (the user left the Resources
//! workspace before its result landed) is rescheduled on return. Before the
//! fix, `auto_loaded_scope` still held the scope that detached fetch was
//! started for, so returning early-returned out of
//! `ensure_active_resource_loaded` and the user was parked on empty rows and
//! the generic press-`r` prompt with nothing in flight.
//!
//! Half (ii): the generation stamp is per `dispatch_effects` *batch*, so a
//! batch cannot invalidate its own later effects. `ResourcesAction::Refresh`
//! with a detail view open emits a list refresh and a detail load in one
//! batch; with a stamp per effect the detail load superseded the list fetch
//! its own batch had just started, and the refreshed rows were always dropped.
//!
//! Frames are written to `NUVEM_EVIDENCE_DIR` when it is set, and only then: this
//! test never picks a fallback output directory and never asserts on the
//! contents of one, so concurrent runs of the suite on one machine cannot
//! collide (a known failure mode). The assertions always run.

use std::fs;
use std::path::PathBuf;

use nuvem::app::state::resources::ResourceLoadPhase;
use nuvem::app::{Action, AppState, Effect, ResourcesAction, WorkspaceAction};
use nuvem::services::demo::DemoCommandRunner;
use nuvem::services::runtime::session::RuntimeSession;
use ratatui::{Terminal, backend::TestBackend};

const WIDTH: u16 = 160;
const HEIGHT: u16 = 20;

fn evidence_dir() -> Option<PathBuf> {
    // No fallback: with NUVEM_EVIDENCE_DIR unset this test writes nothing at all
    // rather than reaching for a shared, machine-global path.
    let dir = std::env::var("NUVEM_EVIDENCE_DIR").ok()?;
    let path = PathBuf::from(dir);
    fs::create_dir_all(&path).ok()?;
    Some(path)
}

fn write_frame(name: &str, caption: &str, lines: &[String]) {
    let Some(dir) = evidence_dir() else {
        return;
    };
    let width = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0);
    let mut body = format!("{caption}\n\n+{}+\n", "-".repeat(width));
    for line in lines {
        let pad = width - line.chars().count();
        body.push_str(&format!("|{line}{}|\n", " ".repeat(pad)));
    }
    body.push_str(&format!("+{}+\n", "-".repeat(width)));
    fs::write(dir.join(format!("{name}.txt")), body).expect("write ascii frame");
}

fn render_lines(session: &RuntimeSession) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(WIDTH, HEIGHT)).expect("terminal");
    terminal
        .draw(|frame| nuvem::ui::render(frame, session))
        .expect("render");
    let buffer = terminal.backend().buffer();
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer.cell((x, y)).expect("cell").symbol().to_string())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect()
}

fn screen(session: &RuntimeSession) -> String {
    render_lines(session).join("\n")
}

fn instance_names(session: &RuntimeSession) -> Vec<String> {
    session
        .app()
        .root
        .resources
        .instances()
        .iter()
        .map(|instance| instance.name.clone())
        .collect()
}

/// Whether the active service's list fetch is in flight. `begin_load` runs
/// synchronously inside `dispatch_background_refresh`, so this flips before
/// `dispatch_action` returns: a race-free signal that a fetch WAS scheduled.
fn fetch_in_flight(session: &RuntimeSession) -> bool {
    matches!(
        session.app().root.resources.active_load_phase(),
        ResourceLoadPhase::Loading { .. }
    )
}

/// A session wired to the demo fixtures, with the demo `dev` configuration
/// active — loaded through the runner, not hand-fed as state.
async fn demo_session() -> RuntimeSession {
    let mut session = RuntimeSession::new_with_runner_for_test(
        AppState::new(),
        Box::new(DemoCommandRunner::new()),
    );
    session
        .dispatch_action(Action::Effect(Effect::LoadConfigurations))
        .await;
    session
}

/// Half (i), as a reviewer sees it: enter Resources, leave mid-fetch so the
/// result is dropped, come back — and the rows arrive without a keypress.
#[tokio::test]
async fn returning_after_a_detached_list_fetch_refetches_the_rows() {
    let mut session = demo_session().await;

    // Entering Resources auto-fires the first GCE list fetch.
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;
    assert!(fetch_in_flight(&session));
    write_frame(
        "01-resources-entered-list-fetch-in-flight",
        "1. Entering the Resources workspace auto-fires the GCE instance list fetch. Nothing has \
landed yet, so the list is empty and the operation line shows the load in progress.",
        &render_lines(&session),
    );

    // The user leaves before the result lands. That bumps `scope_generation`,
    // so the in-flight result loses the guard in `process_next_effect_result`
    // and is dropped: the fetch is detached.
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Tunnels))
        .await;
    assert!(session.process_next_effect_result().await);
    assert_eq!(
        instance_names(&session),
        Vec::<String>::new(),
        "the detached fetch's rows must not be applied to a workspace the user has left"
    );
    assert_eq!(
        session.app().root.resources.active_load_phase(),
        &ResourceLoadPhase::Idle,
        "and the abandoned load must resolve its spinner back to Idle"
    );
    write_frame(
        "02-left-mid-fetch-result-dropped",
        "2. The user switched to Tunnels before the fetch finished. The result arrived for a \
workspace that is no longer on screen and was dropped; the load phase resolved back to Idle, so \
no spinner is left behind for a fetch nobody will apply.",
        &render_lines(&session),
    );

    // Returning is where the defect lives: the drop site forgot the
    // `auto_loaded_scope` record for the abandoned fetch, so the auto-load
    // re-arms instead of early-returning on a scope that was never satisfied.
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;
    assert!(
        fetch_in_flight(&session),
        "returning to Resources after a dropped fetch must schedule a new one"
    );
    let returned = render_lines(&session);
    write_frame(
        "03-returned-refetch-scheduled",
        "3. Back in Resources. BEFORE THE FIX this frame was the end of the story: empty rows and \
the press-r prompt, with nothing in flight, because auto_loaded_scope still recorded the scope \
the detached fetch had been started for. Now the refetch is already running.",
        &returned,
    );

    assert!(session.process_next_effect_result().await);
    let names = instance_names(&session);
    assert_eq!(
        names,
        vec!["api-dev-1".to_string(), "worker-dev-1".to_string()],
        "the rescheduled fetch's rows must land"
    );
    let loaded = screen(&session);
    assert!(
        loaded.contains("api-dev-1") && loaded.contains("worker-dev-1"),
        "and they must be on screen:\n{loaded}"
    );
    write_frame(
        "04-returned-rows-landed",
        "4. The rescheduled fetch's rows land on their own. The user pressed no keys between \
frames 2 and 4 beyond navigating back to Resources.",
        &render_lines(&session),
    );
}

/// Half (ii), as a reviewer sees it: refreshing with a detail view open used to
/// drop the list rows its own batch had just fetched.
#[tokio::test]
async fn refreshing_with_a_detail_view_open_keeps_the_refreshed_rows() {
    let mut session = demo_session().await;

    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;
    assert!(session.process_next_effect_result().await);
    assert_eq!(
        instance_names(&session),
        vec!["api-dev-1".to_string(), "worker-dev-1".to_string()],
    );

    // Open a detail view, so `ResourcesAction::Refresh` emits TWO resource
    // effects in one batch: the list refresh and the detail reload.
    session
        .dispatch_action(Action::Resources(ResourcesAction::OpenSelected))
        .await;
    assert!(
        session.app().root.resources.detail().is_some(),
        "the detail view must be open for the refresh batch to contain two resource effects"
    );
    write_frame(
        "05-detail-open-before-refresh",
        "5. A GCE instance detail view is open over a loaded list. Pressing r here emits a list \
refresh AND a detail reload in a single dispatch_effects batch.",
        &render_lines(&session),
    );

    session
        .dispatch_action(Action::Resources(ResourcesAction::Refresh))
        .await;
    assert!(
        fetch_in_flight(&session),
        "the refresh batch must have started a background list fetch"
    );
    write_frame(
        "06-refresh-dispatched-list-fetch-in-flight",
        "6. `r` pressed. The batch's detail reload has already run inline — that is the operation \
the status line names — while the list fetch it was dispatched alongside is still in flight in \
the background. That ordering is exactly what used to sink the list result.",
        &render_lines(&session),
    );

    assert!(session.process_next_effect_result().await);
    assert_eq!(
        instance_names(&session),
        vec!["api-dev-1".to_string(), "worker-dev-1".to_string()],
        "the list result of a refresh batch must survive that same batch's detail load"
    );

    session
        .dispatch_action(Action::Resources(ResourcesAction::CloseDetail))
        .await;
    let after = screen(&session);
    assert!(
        after.contains("api-dev-1") && after.contains("worker-dev-1"),
        "the refreshed rows must be on screen after closing the detail:\n{after}"
    );
    // The load-bearing assertion, and the one that fails on the old code: the
    // status line reports the *list* refresh completing. The demo fixtures are
    // static, so identical rows before and after prove nothing on their own —
    // this line does, because it is only reached from the apply site. With a
    // per-effect stamp the batch's detail load superseded the list fetch the
    // same batch had just started, the list result was dropped, and the last
    // operation the user ever saw complete was "GCE detail": the refresh they
    // asked for finished silently having applied nothing, leaving whatever
    // rows were already there. Against a real project those rows are stale.
    assert!(
        after.contains("[OK] GCE instances"),
        "the refreshed list load must be the operation that completes:\n{after}"
    );
    write_frame(
        "07-list-after-refresh-with-detail-open",
        "7. The list behind the detail view once that refresh lands, with the status line reading \
\"[OK] GCE instances\" — the list refresh itself completed and its rows were applied. BEFORE \
THE FIX the generation stamp moved once per effect, so the batch's detail load superseded the \
list fetch the same batch had just started: the list result was dropped, this line instead read \
\"[OK] GCE detail\", and the rows on screen were whatever the previous load had left. The stamp \
is now per batch, so both effects carry the same generation and neither invalidates the other.",
        &render_lines(&session),
    );
}
