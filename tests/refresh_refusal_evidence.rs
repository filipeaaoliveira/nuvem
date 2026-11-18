//! Visual evidence — `r` in the log viewer says why it could not
//! load when the gcloud context stopped resolving.
//!
//! `tests/reducer_logging_test.rs` pins the status text and the surviving rows
//! at the reducer. This binary answers the reviewer's other question: what does
//! the operator actually see. Both frames go through the real key -> keymap ->
//! reducer -> effect -> `nuvem::ui::render` path against a stand-in `gcloud`,
//! so the "before" and "after" screens differ only in what the status line
//! says — the rows are byte-for-byte the same, which is exactly the confusion
//! the message exists to end.
//!
//! Frames are written only when `NUVEM_EVIDENCE_DIR` is set, and are written
//! before the assertions run so a frame is captured even on a failing run.

use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use nuvem::app::AppState;
use nuvem::app::EffectResult;
use nuvem::app::state::root::Workspace;
use nuvem::input::keymap::action_for_key;
use nuvem::services::gcloud::cli::{CommandError, CommandOutput, CommandRunner, GcloudCommand};
use nuvem::services::runtime::input_context_for_test;
use nuvem::services::runtime::session::RuntimeSession;
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::support::frame_svg;

use frame_svg::buffer_to_svg;

const WIDTH: u16 = 120;
const HEIGHT: u16 = 22;

const REFUSAL: &str = "No active gcloud context to refresh logs from";

/// The page the operator is mid-read of when the context disappears: the rows
/// that must still be on screen after the refused refresh.
const CORPUS: &[(&str, &str, &str)] = &[
    (
        "ERROR",
        "cloud_run_revision",
        "checkout: upstream returned 502 after 3 retries",
    ),
    (
        "CRITICAL",
        "cloud_run_revision",
        "payments: database pool exhausted, shedding load",
    ),
    (
        "WARNING",
        "gce_instance",
        "disk usage on / is at 82% (threshold 80%)",
    ),
    (
        "INFO",
        "k8s_container",
        "scaled deployment api from 3 to 5 replicas",
    ),
];

/// Stand-in for the `gcloud` binary, answering `logging read` from [`CORPUS`]
/// and counting the calls, so "the fetch was never dispatched" is observable
/// rather than assumed.
struct FakeGcloud {
    calls: Arc<Mutex<usize>>,
}

impl CommandRunner for FakeGcloud {
    fn run(&self, _command: GcloudCommand) -> Result<CommandOutput, CommandError> {
        *self.calls.lock().unwrap() += 1;
        let rows: Vec<String> = CORPUS
            .iter()
            .enumerate()
            .map(|(index, (severity, resource_type, message))| {
                format!(
                    r#"{{"insertId":"log-{index:03}","timestamp":"2026-07-22T11:5{index}:00.000000000Z","severity":"{severity}","logName":"projects/acme-prod/logs/stdout","resource":{{"type":"{resource_type}","labels":{{"instance_id":"6081246938160990230"}}}},"textPayload":"{message}","labels":{{"env":"prod"}}}}"#
                )
            })
            .collect();
        Ok(CommandOutput {
            stdout: format!("[{}]", rows.join(",")),
            stderr: String::new(),
        })
    }
}

fn evidence_dir() -> Option<PathBuf> {
    let dir = std::env::var("NUVEM_EVIDENCE_DIR").ok()?;
    let path = PathBuf::from(dir);
    fs::create_dir_all(&path).ok()?;
    Some(path)
}

fn text(buffer: &Buffer) -> String {
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol().to_string())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn write_frame(name: &str, title: &str, caption: &str, buffer: &Buffer) {
    let Some(dir) = evidence_dir() else {
        eprintln!("NUVEM_EVIDENCE_DIR not set; skipping evidence render");
        return;
    };
    fs::write(
        dir.join(format!("{name}.svg")),
        buffer_to_svg(buffer, title, caption),
    )
    .expect("write svg frame");
    fs::write(
        dir.join(format!("{name}.txt")),
        format!("{title}\n{caption}\n\n{}\n", text(buffer)),
    )
    .expect("write ascii frame");
}

struct Viewer {
    session: RuntimeSession,
    calls: Arc<Mutex<usize>>,
}

impl Viewer {
    fn new() -> Viewer {
        let mut app = AppState::new();
        app.root.configurations.set_rows(vec![GcloudConfiguration {
            name: GcloudConfigurationName::new("prod"),
            is_active: true,
            account: Some("sre@example.com".to_string()),
            project: Some("acme-prod".to_string()),
            region: Some("europe-west1".to_string()),
            zone: Some("europe-west1-b".to_string()),
        }]);
        app.root.workspace = Workspace::Logging;
        let calls = Arc::new(Mutex::new(0));
        let session = RuntimeSession::new_with_runner_for_test(
            app,
            Box::new(FakeGcloud {
                calls: calls.clone(),
            }),
        );
        Viewer { session, calls }
    }

    /// One real `r` key event, resolved through the real input context and
    /// keymap the way the event loop does.
    async fn press_refresh(&mut self) {
        let key = KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE);
        let context = input_context_for_test(&self.session, Rect::new(0, 0, WIDTH, HEIGHT));
        let action = action_for_key(key, &context).expect("`r` is bound in the log viewer");
        self.session.dispatch_action(action).await;
    }

    /// Applies whatever the press queued, so the screenshot shows the server's
    /// answer instead of the spinner. A refused refresh queues nothing.
    async fn drain(&mut self) {
        while let Ok(true) = tokio::time::timeout(
            std::time::Duration::from_millis(200),
            self.session.process_next_effect_result(),
        )
        .await
        {}
    }

    fn draw(&self) -> Buffer {
        let mut terminal = Terminal::new(TestBackend::new(WIDTH, HEIGHT)).expect("terminal init");
        terminal
            .draw(|frame| nuvem::ui::render(frame, &self.session))
            .expect("render");
        terminal.backend().buffer().clone()
    }

    fn calls(&self) -> usize {
        *self.calls.lock().unwrap()
    }
}

/// The operator's walkthrough: a refresh that loads, then the same key press
/// against a context that no longer resolves.
#[tokio::test]
async fn a_refused_refresh_names_the_missing_context_on_screen() {
    let mut viewer = Viewer::new();

    // ---- 1. `r` with a context that resolves: the page loads -------------
    viewer.press_refresh().await;
    viewer.drain().await;
    let loaded = viewer.draw();
    let calls_after_load = viewer.calls();
    write_frame(
        "01-refresh-loads",
        "1. `r` with a resolving gcloud context",
        "The fetch is dispatched and the page comes back; the status line carries no \
         refusal, because there was none.",
        &loaded,
    );

    // ---- 2. The context stops resolving, and `r` is pressed again --------
    // A `gcloud config` reload that no longer lists the active configuration,
    // which is how the context disappears under a running viewer.
    viewer
        .session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: Vec::new(),
        })
        .await;
    viewer.press_refresh().await;
    viewer.drain().await;
    let refused = viewer.draw();
    write_frame(
        "02-refresh-refused",
        "2. The same `r` after the gcloud context stopped resolving",
        "No fetch is dispatched and the rows deliberately stay put, so the only \
         difference from frame 1 is the status line: \"No active gcloud context to refresh \
         logs from\".",
        &refused,
    );

    let loaded_text = text(&loaded);
    let refused_text = text(&refused);

    assert_eq!(
        calls_after_load, 1,
        "the first `r` must really have run `gcloud logging read`:\n{loaded_text}"
    );
    assert!(
        !loaded_text.contains(REFUSAL),
        "a refresh that loaded reported a refusal that did not happen:\n{loaded_text}"
    );
    assert_eq!(
        viewer.calls(),
        calls_after_load,
        "the refused refresh dispatched a fetch after all, so there was nothing to report"
    );
    assert!(
        refused_text.contains(REFUSAL),
        "the refused refresh left the screen silent about why nothing loaded:\n{refused_text}"
    );
    for (_, _, message) in CORPUS {
        let fragment: String = message.chars().take(28).collect();
        assert!(
            refused_text.contains(&fragment),
            "the refused refresh cost the operator the row it was explaining: {fragment:?} \
             is gone from\n{refused_text}"
        );
    }
}
