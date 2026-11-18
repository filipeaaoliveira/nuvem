//! Evidence — a plugin label that NAMES a credential survives to the
//! operator, while a credential a failure actually CARRIES does not.
//!
//! The unit tests pin `sanitize_detail` token by token. This binary answers
//! the reviewer's other question: what does the operator see. It walks the
//! whole path the defect was reported on — a `[[plugins]]` entry loaded
//! through the real config parser, its `label = "Describe {name}"` rendered
//! against a real Secret Manager row called `db-password`, the plugin
//! confirmed through the real keymap and reducer, a failing `CommandRunner`
//! standing in for gcloud, and finally `Y` copying the diagnostic through a
//! fake clipboard so what lands on the pasteboard is observable rather than
//! assumed.
//!
//! The copied diagnostic is where the two halves meet: its `command:` line is
//! the operator's own label, which used to read `command: [redacted]`, and its
//! `detail:` line is gcloud's stderr, which still redacts the API key gcloud
//! echoed back. Frames are written only when `NUVEM_EVIDENCE_DIR` is set, and are
//! written before the assertions run so a failing run still leaves the frame
//! that explains it.

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use nuvem::app::{Action, AppState, EffectResult, ResourcesAction, WorkspaceAction};
use nuvem::config::plugins::Plugins;
use nuvem::config::problems::ConfigProblems;
use nuvem::input::keymap::action_for_key;
use nuvem::services::gcloud::cli::{CommandError, CommandOutput, CommandRunner, GcloudCommand};
use nuvem::services::runtime::clipboard::ClipboardWriter;
use nuvem::services::runtime::input_context_for_test;
use nuvem::services::runtime::session::RuntimeSession;
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use nuvem_domain::gcp::resource::{ResourceServiceKind, Secret};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use tempfile::NamedTempFile;

use crate::support::frame_svg::buffer_to_svg;

const WIDTH: u16 = 140;
const HEIGHT: u16 = 26;

/// The row the ticket names: an ordinary Secret Manager secret whose own name
/// contains a credential word. Nothing about it is a credential; it is a
/// resource identifier an operator reads off their own project.
const ROW: &str = "db-password";

/// What gcloud writes to stderr when it echoes the flag it choked on. The key
/// carries a value, so this half must not survive.
const CARRIED_CREDENTIAL: &str = "--api-key=AIzaSyExampleNotARealKey";

const PLUGIN_FILE: &str = "version = 1\n\n\
     [[plugins]]\n\
     id       = \"describe\"\n\
     scope    = \"secrets\"\n\
     key      = \"d\"\n\
     label    = \"Describe {name}\"\n\
     args     = [\"secrets\", \"describe\", \"{name}\"]\n";

/// A gcloud that always fails, with stderr that names the plugin's row and
/// carries a credential-valued flag.
struct FailingGcloud;

impl CommandRunner for FailingGcloud {
    fn run(&self, _command: GcloudCommand) -> Result<CommandOutput, CommandError> {
        Err(CommandError::Failed {
            stdout: String::new(),
            stderr: format!(
                "ERROR: (gcloud.secrets.describe) PERMISSION_DENIED on {ROW} \
                 [invalid credential {CARRIED_CREDENTIAL}]"
            ),
            exit_code: Some(1),
        })
    }
}

#[derive(Clone, Default)]
struct FakeClipboard {
    values: Arc<Mutex<Vec<String>>>,
}

impl ClipboardWriter for FakeClipboard {
    fn set_text(&self, value: &str) -> Result<(), String> {
        self.values.lock().unwrap().push(value.to_string());
        Ok(())
    }
}

fn evidence_dir() -> Option<PathBuf> {
    let path = PathBuf::from(std::env::var("NUVEM_EVIDENCE_DIR").ok()?);
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

fn write_text(name: &str, body: &str) {
    let Some(dir) = evidence_dir() else {
        return;
    };
    fs::write(dir.join(format!("{name}.txt")), body).expect("write text evidence");
}

/// The plugin set, built the only way one is ever built: through the real
/// config parser over a real file, so the label under test is a `Template`
/// the config layer accepted rather than a string this test wrote.
fn describe_plugin() -> (Plugins, NamedTempFile) {
    let mut file = NamedTempFile::new().expect("a temp plugin file");
    file.write_all(PLUGIN_FILE.as_bytes())
        .expect("the fixture writes");
    let mut problems = ConfigProblems::default();
    let plugins = Plugins::load(file.path(), &mut problems);
    assert!(
        problems.is_empty(),
        "the fixture plugin must load cleanly: {:?}",
        problems.messages()
    );
    (plugins, file)
}

struct Operator {
    session: RuntimeSession,
    clipboard: FakeClipboard,
    _plugin_file: NamedTempFile,
}

impl Operator {
    async fn at_the_secret_list() -> Operator {
        let (plugins, plugin_file) = describe_plugin();
        let mut app = AppState::new();
        app.root.plugins = plugins;

        let clipboard = FakeClipboard::default();
        let mut session = RuntimeSession::new_with_runner_and_clipboard_for_test(
            app,
            Box::new(FailingGcloud),
            Box::new(clipboard.clone()),
        );
        session
            .apply_effect_result(EffectResult::ConfigurationsLoaded {
                configurations: vec![GcloudConfiguration {
                    name: GcloudConfigurationName::new("dev"),
                    is_active: true,
                    account: Some("sre@example.com".to_string()),
                    project: Some("dev-project".to_string()),
                    region: Some("europe-west1".to_string()),
                    zone: Some("europe-west1-b".to_string()),
                }],
            })
            .await;
        session
            .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
            .await;
        session
            .dispatch_action(Action::Resources(ResourcesAction::SelectService(
                ResourceServiceKind::SecretManagerSecrets,
            )))
            .await;
        let context = session
            .app()
            .root
            .current_context()
            .expect("the dev configuration resolves")
            .clone();
        session
            .apply_effect_result(EffectResult::SecretsLoaded {
                context,
                secrets: vec![Secret {
                    name: ROW.to_string(),
                    create_time: Some("2026-01-09T10:12:00Z".to_string()),
                    labels: Vec::new(),
                    replication: "Automatic".to_string(),
                }],
            })
            .await;

        Operator {
            session,
            clipboard,
            _plugin_file: plugin_file,
        }
    }

    async fn press(&mut self, code: KeyCode) {
        let key = KeyEvent::new(code, KeyModifiers::NONE);
        let context = input_context_for_test(&self.session, Rect::new(0, 0, WIDTH, HEIGHT));
        let action =
            action_for_key(key, &context).unwrap_or_else(|| panic!("{code:?} is bound here"));
        self.session.dispatch_action(action).await;
    }

    async fn type_text(&mut self, query: &str) {
        for ch in query.chars() {
            self.press(KeyCode::Char(ch)).await;
        }
    }

    fn draw(&self) -> Buffer {
        let mut terminal = Terminal::new(TestBackend::new(WIDTH, HEIGHT)).expect("terminal init");
        terminal
            .draw(|frame| nuvem::ui::render(frame, &self.session))
            .expect("render");
        terminal.backend().buffer().clone()
    }

    fn copied(&self) -> Vec<String> {
        self.clipboard.values.lock().unwrap().clone()
    }

    /// Drain the effect-result channel until the run this walk started has
    /// settled. Deterministic: the runtime sends exactly one result per run,
    /// so a still-pending entry means one is on its way.
    async fn settle(&mut self) {
        let mut drained = 0;
        while self.session.app().root.system.operations.pending_count() > 0 {
            assert!(drained < 8, "the plugin's entry was still pending");
            assert!(
                self.session.process_next_effect_result().await,
                "the result channel closed with the plugin's entry still pending"
            );
            drained += 1;
        }
    }
}

/// The reported defect, end to end. `Describe db-password` is the operator's
/// own label; the substring rule used to redact it on the way into
/// the diagnostic, so the copied text read `command: [redacted]` and the
/// operator could not tell which of their plugins had failed.
#[tokio::test]
async fn a_label_naming_a_secret_survives_the_copied_diagnostic_while_a_carried_key_does_not() {
    let mut operator = Operator::at_the_secret_list().await;

    operator.press(KeyCode::Char(':')).await;
    operator.type_text("describe").await;
    operator.press(KeyCode::Enter).await;
    let confirmation = operator.draw();
    write_frame(
        "01-confirmation",
        "The plugin an operator confirms",
        "`Describe db-password`: the label is templated against the Secret Manager row",
        &confirmation,
    );
    assert!(
        text(&confirmation).contains("Describe db-password"),
        "the modal must name the row the plugin runs against: {}",
        text(&confirmation)
    );

    operator.press(KeyCode::Enter).await;
    operator.settle().await;

    let failed = operator.draw();
    write_frame(
        "02-failure",
        "The failure the operator reads",
        "the status line and the operations log name the plugin, not `[redacted]`",
        &failed,
    );

    operator.press(KeyCode::Char('Y')).await;
    let copied = operator.copied();
    let diagnostic = copied
        .last()
        .expect("Y must copy the last diagnostic")
        .clone();
    write_text(
        "03-copied-diagnostic",
        &format!(
            "What `Y` puts on the operator's clipboard\n\
             gcloud stderr was: ERROR: (gcloud.secrets.describe) PERMISSION_DENIED on {ROW} \
             [invalid credential {CARRIED_CREDENTIAL}]\n\n{diagnostic}\n"
        ),
    );

    let command_line = diagnostic
        .lines()
        .find(|line| line.starts_with("command: "))
        .expect("the copied diagnostic carries a command line");
    assert_eq!(
        command_line, "command: Describe db-password",
        "the operator's own label names a credential; it does not carry one"
    );

    let detail_line = diagnostic
        .lines()
        .find(|line| line.starts_with("detail: "))
        .expect("the copied diagnostic carries a detail line");
    assert!(
        !detail_line.contains("AIzaSyExampleNotARealKey"),
        "a flag whose value IS a credential must still be redacted: {detail_line:?}"
    );
    assert!(
        detail_line.contains("[redacted]"),
        "the carried credential must be replaced, not dropped silently: {detail_line:?}"
    );
    assert!(
        detail_line.contains("PERMISSION_DENIED") && detail_line.contains(ROW),
        "the rest of gcloud's own words, including the row it names, must survive: \
         {detail_line:?}"
    );
}
