//! Visual evidence — a validated `[[plugins]]` entry is reachable
//! from the command palette and shows its exact argv before anything happens
//! to it.
//!
//! The unit and reducer tests pin the argv, the labelling and the refusals.
//! This binary answers the reviewer's other question: what does the operator
//! actually see. It walks the README's own end-to-end scenario ("Take the
//! `describe` plugin above, with the Compute Engine list in view, the row
//! `web-01` selected, and the `dev` gcloud configuration active") through the
//! real key -> keymap -> reducer -> effect -> `nuvem::ui::render` path, with
//! the plugin loaded from a real `plugins.toml` through the real config
//! parser, and a fake clipboard standing in for the pasteboard so the
//! clipboard claims are observable rather than assumed.
//!
//! What the argv is shown *in* has moved since: submitting a plugin opened the
//! copy-only `ResourceCommandPreview` when this file was written, and opens the
//! plugin confirmation now. The scenario and the assertions about the rendered
//! command line are the same either way — they are what this evidence is for —
//! so they follow the modal rather than being retired with it.
//!
//! Frames are written only when `NUVEM_EVIDENCE_DIR` is set, and are written
//! before the assertions run so a frame is captured even on a failing run.

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use nuvem::app::state::root::Workspace;
use nuvem::app::{Action, AppState, EffectResult, ResourcesAction, WorkspaceAction};
use nuvem::config::plugins::Plugins;
use nuvem::config::problems::ConfigProblems;
use nuvem::input::keymap::action_for_key;
use nuvem::services::gcloud::cli::{CommandError, CommandOutput, CommandRunner, GcloudCommand};
use nuvem::services::runtime::clipboard::ClipboardWriter;
use nuvem::services::runtime::input_context_for_test;
use nuvem::services::runtime::session::RuntimeSession;
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use nuvem_domain::gcp::resource::{ComputeInstance, ResourceStatus};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use tempfile::NamedTempFile;

use crate::support::frame_svg::buffer_to_svg;

const WIDTH: u16 = 140;
const HEIGHT: u16 = 26;

/// The README's own example file, verbatim in shape: one Compute-scoped
/// plugin whose label and argv are both templated against the row.
const PLUGIN_FILE: &str = "version = 1\n\n\
     [[plugins]]\n\
     id       = \"describe\"\n\
     scope    = \"compute\"\n\
     key      = \"d\"\n\
     label    = \"Describe {name}\"\n\
     args     = [\"compute\", \"instances\", \"describe\", \"{name}\", \"--zone\", \"{zone}\"]\n";

/// The same file with a verb nuvem never vouched for, for the `--read-only`
/// evidence. The mode admits a plugin whose declared command path it can prove
/// only reads, so the plugin that gets refused has to be one that
/// declares something else — the README's `describe` now runs under the flag.
const MUTATING_PLUGIN_FILE: &str = "version = 1\n\n\
     [[plugins]]\n\
     id       = \"delete\"\n\
     scope    = \"compute\"\n\
     key      = \"x\"\n\
     label    = \"Delete {name}\"\n\
     args     = [\"compute\", \"instances\", \"delete\", \"{name}\", \"--zone\", \"{zone}\"]\n";

/// The line the README promises the preview shows, including the
/// `--configuration dev` nuvem injects rather than the author writing it.
const EXPECTED_COMMAND: &str =
    "gcloud --configuration dev compute instances describe web-01 --zone europe-west1-b";

struct NoGcloud;

impl CommandRunner for NoGcloud {
    fn run(&self, command: GcloudCommand) -> Result<CommandOutput, CommandError> {
        panic!("this slice runs no process; a gcloud call escaped: {command:?}");
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

/// The plugin set, built the only way a plugin is ever built: through the real
/// config parser over a real file. A fixture that skipped validation would be
/// previewing a `Plugin` the config layer would never hand over.
fn plugin_from(body: &str) -> (Plugins, NamedTempFile) {
    let mut file = NamedTempFile::new().expect("a temp plugin file");
    file.write_all(body.as_bytes()).expect("the fixture writes");
    let mut problems = ConfigProblems::default();
    let plugins = Plugins::load(file.path(), &mut problems);
    assert!(
        problems.is_empty(),
        "the README's own example must load cleanly: {:?}",
        problems.messages()
    );
    assert_eq!(plugins.len(), 1, "the fixture plugin must load");
    (plugins, file)
}

/// The README's operator, at the Compute list with `web-01` selected and the
/// `describe` plugin loaded. `pub(crate)` because the execution evidence walks the
/// same operator through the same palette to a different modal: a second
/// fixture built alongside this one could drift from it, and then the two
/// binaries would be describing two different applications.
pub(crate) struct Operator {
    pub(crate) session: RuntimeSession,
    clipboard: FakeClipboard,
    _plugin_file: NamedTempFile,
}

impl Operator {
    /// This file's operator: the same walk with a runner that panics, because
    /// nothing here is meant to reach a process.
    pub(crate) async fn at_the_compute_list(read_only: bool) -> Operator {
        Operator::at_the_compute_list_with_runner(read_only, Box::new(NoGcloud)).await
    }

    /// The same operator with a plugin that declares a verb nuvem never
    /// vouched for, for the read-only evidence here and in the execution file. It is
    /// the same walk: what changes is the declaration `--read-only` reads.
    pub(crate) async fn at_the_compute_list_with_a_mutating_plugin(read_only: bool) -> Operator {
        Operator::at_the_compute_list_with(read_only, Box::new(NoGcloud), MUTATING_PLUGIN_FILE)
            .await
    }

    /// The same operator with a caller-supplied runner, for the execution
    /// evidence that walks this palette all the way to a running child. The
    /// fixture is shared rather than copied for the reason the type is
    /// `pub(crate)`: two fixtures would drift, and then the binary that proves
    /// a plugin runs and the binary that proves what it shows would be
    /// describing two different applications.
    pub(crate) async fn at_the_compute_list_with_runner(
        read_only: bool,
        runner: Box<dyn CommandRunner + Send + Sync>,
    ) -> Operator {
        Operator::at_the_compute_list_with(read_only, runner, PLUGIN_FILE).await
    }

    /// The walk itself, over whichever plugin file the caller is about.
    async fn at_the_compute_list_with(
        read_only: bool,
        runner: Box<dyn CommandRunner + Send + Sync>,
        plugin_body: &str,
    ) -> Operator {
        let (plugins, plugin_file) = plugin_from(plugin_body);
        let mut app = AppState::new();
        app.root.plugins = plugins;
        app.root.read_only = read_only;

        let clipboard = FakeClipboard::default();
        let mut session = RuntimeSession::new_with_runner_and_clipboard_for_test(
            app,
            runner,
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
        let context = session
            .app()
            .root
            .current_context()
            .expect("the dev configuration resolves")
            .clone();
        session
            .apply_effect_result(EffectResult::ComputeInstancesLoaded {
                context,
                instances: vec![
                    instance("api-dev-1", "europe-west1-b"),
                    instance("web-01", "europe-west1-b"),
                ],
            })
            .await;
        // The README's scenario selects `web-01`, the second row: a plugin
        // rendered against the first row would be indistinguishable from one
        // that ignores the cursor.
        session
            .dispatch_action(Action::Resources(ResourcesAction::SelectNext))
            .await;
        assert_eq!(session.app().root.workspace, Workspace::Resources);

        Operator {
            session,
            clipboard,
            _plugin_file: plugin_file,
        }
    }

    /// One real key event, resolved through the real input context and keymap
    /// the way the event loop does.
    pub(crate) async fn press(&mut self, code: KeyCode) {
        self.press_with(code, KeyModifiers::NONE).await;
    }

    /// The same real-keymap press, for the chorded keys — `Ctrl-x` is the
    /// first — that `press` cannot express.
    pub(crate) async fn press_with(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        let key = KeyEvent::new(code, modifiers);
        let context = input_context_for_test(&self.session, Rect::new(0, 0, WIDTH, HEIGHT));
        let action =
            action_for_key(key, &context).unwrap_or_else(|| panic!("{code:?} is bound here"));
        self.session.dispatch_action(action).await;
    }

    pub(crate) async fn type_text(&mut self, query: &str) {
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
}

fn instance(name: &str, zone: &str) -> ComputeInstance {
    ComputeInstance {
        id: format!("{name}-id"),
        name: name.to_string(),
        zone: zone.to_string(),
        status: ResourceStatus::Running,
        machine_type: "e2-medium".to_string(),
        internal_ip: Some("10.0.0.2".to_string()),
        external_ip: None,
    }
}

/// The whole walkthrough the README promises: `:` offers the plugin under its
/// own rendered label, and Enter shows the exact argv with the injected
/// `--configuration dev` — with no process run and nothing copied.
#[tokio::test]
async fn a_declared_plugin_is_offered_and_shows_its_exact_argv() {
    let mut operator = Operator::at_the_compute_list(false).await;

    // ---- 1. `:` from the Compute list -----------------------------------
    operator.press(KeyCode::Char(':')).await;
    operator.type_text("describe").await;
    let palette = operator.draw();
    write_frame(
        "01-palette-offers-the-plugin",
        "1. `:` with the GCE list in view and `web-01` selected",
        "The palette lists the plugin under its own `label` template rendered against the \
         selected row — \"Describe web-01\", not the plugin's id and not a placeholder.",
        &palette,
    );

    // ---- 2. Enter opens the confirmation ---------------------------------
    operator.press(KeyCode::Enter).await;
    let preview = operator.draw();
    write_frame(
        "02-preview-shows-the-exact-argv",
        "2. Enter on that row",
        "The confirmation shows the exact command line, `--configuration dev` first — \
         injected by nuvem so the operator reads the configuration on screen rather than \
         whatever their shell has active. Nothing has run.",
        &preview,
    );

    let palette_text = text(&palette);
    let preview_text = text(&preview);

    assert!(
        palette_text.contains("Describe web-01"),
        "the palette did not offer the plugin under its rendered label:\n{palette_text}"
    );
    assert!(
        !palette_text.contains("Plugin command"),
        "the palette showed the placeholder label:\n{palette_text}"
    );
    // The preview line is wider than a modal border allows in one screen row
    // for some terminals; assert the two halves that carry the claim.
    assert!(
        preview_text.contains("--configuration dev compute instances describe web-01"),
        "the preview did not show the injected configuration and the rendered argv:\n\
         {preview_text}"
    );
    assert!(
        preview_text.contains("--zone europe-west1-b"),
        "the preview dropped the trailing argv elements:\n{preview_text}"
    );
    assert!(
        preview_text.contains("Describe web-01"),
        "the preview did not carry the plugin's own label as its heading:\n{preview_text}"
    );
    // Where the copy assertions were. Submitting a plugin used to end at the
    // paste buffer; it now ends at a confirmation the operator answers, so the
    // claim worth pinning is the negative one — reaching the modal writes
    // nothing to the clipboard, and the exact line above is the one the
    // operator is being shown rather than one they have already been handed.
    assert!(
        operator.copied().is_empty(),
        "opening the confirmation wrote to the clipboard: {:?}",
        operator.copied()
    );
    assert_eq!(
        operator
            .session
            .app()
            .root
            .resources
            .plugin_confirmation()
            .expect("the confirmation is what a plugin opens")
            .command_preview(),
        EXPECTED_COMMAND,
        "the modal must hold exactly the line the README promises, whole — the \
         screen assertions above read it in the two halves a modal border wraps \
         it into"
    );
}

/// `--read-only` refuses a plugin whose declared command path nuvem cannot
/// prove only reads. That is the whole rule, so the plugin here
/// declares `delete` rather than the README's `describe` — which the mode now
/// admits, and which would leave this test asserting nothing.
#[tokio::test]
async fn read_only_refuses_the_plugin_and_says_so() {
    let mut operator = Operator::at_the_compute_list_with_a_mutating_plugin(true).await;

    operator.press(KeyCode::Char(':')).await;
    operator.type_text("delete").await;
    operator.press(KeyCode::Enter).await;
    let refused = operator.draw();
    write_frame(
        "04-read-only-refuses",
        "`--read-only`: a plugin that declares a verb nuvem never vouched for",
        "No preview opens and nothing reaches the clipboard: `--read-only` refuses the \
         plugin, naming it by the label the palette row carried.",
        &refused,
    );

    let refused_text = text(&refused);
    assert!(
        refused_text.contains("read-only"),
        "the refusal did not say why nothing happened:\n{refused_text}"
    );
    assert!(
        refused_text.contains("Delete web-01"),
        "the refusal did not name the plugin the operator submitted:\n{refused_text}"
    );
    assert!(
        operator.copied().is_empty(),
        "a refused plugin reached the clipboard: {:?}",
        operator.copied()
    );
}
