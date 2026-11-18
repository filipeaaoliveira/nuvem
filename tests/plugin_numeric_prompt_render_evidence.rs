//! Visual evidence — what the operator actually sees when the
//! prompt admits a `number` and an `integer`.
//!
//! `tests/plugin_numeric_prompt_evidence.rs` pins the parse at the
//! reducer. This file answers the reviewer's other question: the frames go
//! through the real key -> `action_for_key` -> reducer -> `nuvem::ui::render`
//! path, so the refusal, the operator's own text beside it, the key line that
//! stops naming `Enter`, and the argv the corrected value composes are read off
//! the rendered buffer rather than off a `Vec<String>` the modal is built from.
//!
//! Frames are written only when `NUVEM_EVIDENCE_DIR` is set, and are written
//! before the assertions run so a frame is captured even on a failing run.

use std::fs;
use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use nuvem::input::keymap::action_for_key;
use nuvem::services::runtime::input_context_for_test;
use nuvem::services::runtime::session::RuntimeSession;
use nuvem_domain::gcp::resource::ResourceServiceKind;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::support::frame_svg::buffer_to_svg;

use crate::plugin_prefix_evidence::{load, root_with_plugin};

const WIDTH: u16 = 140;
const HEIGHT: u16 = 34;
const ROW: &str = "vm-1";

/// A Compute list with `ROW` selected and one `resize` plugin taking a
/// `string` reason and an `integer` size, loaded through the real parser.
fn session_with(inputs: &str, args: &str) -> RuntimeSession {
    let mut app = root_with_plugin(
        ResourceServiceKind::ComputeInstances,
        'r',
        &["compute", "instances", "describe", "{name}"],
        ROW,
    );
    app.root.plugins = load(&format!(
        "version = 1\n\n\
         [[plugins]]\n\
         id = \"resize\"\n\
         scope = \"compute\"\n\
         key = \"r\"\n\
         label = \"Resize {{name}}\"\n\
         args = {args}\n\
         {inputs}"
    ));
    RuntimeSession::new_for_test(app)
}

async fn press(session: &mut RuntimeSession, key: KeyEvent) {
    let context = input_context_for_test(session, Rect::new(0, 0, WIDTH, HEIGHT));
    if let Some(action) = action_for_key(key, &context) {
        session.dispatch_action(action).await;
    }
}

async fn press_char(session: &mut RuntimeSession, value: char) {
    press(
        session,
        KeyEvent::new(KeyCode::Char(value), KeyModifiers::NONE),
    )
    .await;
}

async fn type_text(session: &mut RuntimeSession, text: &str) {
    for value in text.chars() {
        press_char(session, value).await;
    }
}

fn frame(session: &RuntimeSession) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(WIDTH, HEIGHT)).expect("a test terminal");
    terminal
        .draw(|frame| nuvem::ui::render(frame, session))
        .expect("the frame draws");
    terminal.backend().buffer().clone()
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
    let Ok(dir) = std::env::var("NUVEM_EVIDENCE_DIR") else {
        eprintln!("NUVEM_EVIDENCE_DIR not set; skipping evidence render");
        return;
    };
    let dir = PathBuf::from(dir);
    fs::create_dir_all(&dir).expect("the evidence directory");
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

/// The whole slice on screen: `,` `r` opens a prompt on a plugin that names an
/// `integer` (which the prompt design would have withheld whole), the field says what it
/// wants, a mistyped `1.5` keeps the operator's own characters beside the
/// refusal while `Enter` stays unnamed, and the corrected value composes the
/// argv on the confirmation.
#[tokio::test]
async fn a_numeric_field_is_prompted_refused_and_corrected_on_screen() {
    let mut session = session_with(
        "\n[[plugins.inputs]]\nname = \"reason\"\ntype = \"string\"\n\
         \n[[plugins.inputs]]\nname = \"size\"\ntype = \"integer\"\n",
        "[\"compute\", \"instances\", \"resize\", \"{name}\", \"--reason\", \"{reason}\", \"--size\", \"{size}\"]",
    );

    press_char(&mut session, ',').await;
    press_char(&mut session, 'r').await;
    let opened = frame(&session);
    write_frame(
        "01-prompt-opens-on-a-numeric-plugin",
        "`,` `r` on vm-1",
        "A plugin naming an `integer` used to be withheld; it now opens the prompt, \
         and the numeric field names what it wants.",
        &opened,
    );
    let screen = text(&opened);
    assert!(
        screen.contains("size: (needs a whole number)"),
        "the integer field must ask for a whole number:\n{screen}"
    );

    type_text(&mut session, "quarterly review").await;
    press(
        &mut session,
        KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
    )
    .await;
    type_text(&mut session, "1.5").await;
    let refused = frame(&session);
    write_frame(
        "02-a-mistyped-integer-is-refused",
        "typed `1.5` into the integer field",
        "The operator's own text stays beside the refusal, the string field \
         keeps what was typed into it, and the key line does not name `Enter`.",
        &refused,
    );
    let screen = text(&refused);
    assert!(
        screen.contains("size: 1.5 (needs a whole number)"),
        "the refusal must keep the text the operator corrects:\n{screen}"
    );
    assert!(
        screen.contains("reason: quarterly review"),
        "a refusal in one field must not disturb another:\n{screen}"
    );

    press(
        &mut session,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    )
    .await;
    assert!(
        session.app().root.resources.plugin_confirmation().is_none(),
        "Enter on an unparseable field must compose nothing"
    );

    press(
        &mut session,
        KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE),
    )
    .await;
    press(
        &mut session,
        KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE),
    )
    .await;
    type_text(&mut session, "00").await;
    let corrected = frame(&session);
    write_frame(
        "03-a-corrected-integer-offers-enter",
        "corrected the field to `100`",
        "A value the rule accepts renders bare, and `Enter` is named again.",
        &corrected,
    );
    let screen = text(&corrected);
    assert!(
        screen.contains("size: 100") && !screen.contains("whole number"),
        "an accepted value renders as itself:\n{screen}"
    );

    press(
        &mut session,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    )
    .await;
    let confirmed = frame(&session);
    write_frame(
        "04-the-collected-integer-reaches-the-argv",
        "Enter on the satisfied prompt",
        "The confirmation the operator approves carries the collected value.",
        &confirmed,
    );
    let screen = text(&confirmed);
    assert!(
        screen.contains("--size 100") && screen.contains("--reason 'quarterly review'"),
        "the argv on the confirmation must carry the collected values:\n{screen}"
    );
}

/// A `number` reaches the confirmation as the characters that were typed:
/// `0.50` is not re-rendered as `0.5`, so the screen the operator approves
/// shows the command that will run.
#[tokio::test]
async fn a_typed_number_reaches_the_confirmation_verbatim() {
    let mut session = session_with(
        "\n[[plugins.inputs]]\nname = \"share\"\ntype = \"number\"\n",
        "[\"compute\", \"instances\", \"resize\", \"{name}\", \"--share\", \"{share}\"]",
    );

    press_char(&mut session, ',').await;
    press_char(&mut session, 'r').await;
    type_text(&mut session, "0.50").await;
    press(
        &mut session,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    )
    .await;

    let confirmed = frame(&session);
    write_frame(
        "05-a-typed-number-is-shown-as-typed",
        "typed `0.50` into a `number` field",
        "A declared default is stored canonically (0.50 -> 0.5); a typed value \
         is shown and sent as the operator entered it.",
        &confirmed,
    );
    let screen = text(&confirmed);
    assert!(
        screen.contains("--share 0.50"),
        "the confirmation shows the characters that will run:\n{screen}"
    );
}
