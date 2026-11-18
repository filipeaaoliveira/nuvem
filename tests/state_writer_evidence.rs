//! Evidence: what the state writer actually does on disk, and what a
//! user sees when it cannot do it.
//!
//! The unit tests in `src/config/state.rs` pin the failure paths; none of them
//! shows a reviewer the bytes that land beside a hand-written `config.toml`, or
//! the screen a user is looking at when state cannot be read or written. This
//! binary records both:
//!
//! 1. A real save next to a hand-edited `config.toml`, with both files dumped
//!    verbatim — the config is byte-for-byte untouched, comments and key order
//!    included (decision 1).
//! 2. The same destination saved twice, with the inode recorded either side and
//!    the directory listed after — the replacement is a rename and leaves no
//!    staging file behind (decision 2).
//! 3. The three degrade-loudly paths (corrupt file, future version, unwritable
//!    directory) driven through the real `problems.rs` mechanism and painted by
//!    the real `nuvem::ui::render`, so the `Config problems` block a user reads
//!    is the recorded one (decisions 3 and 4).
//!
//! Assertions run unconditionally; artifacts are written only when
//! `NUVEM_EVIDENCE_DIR` is set, and only under that directory.

use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::PathBuf;

use nuvem::app::reducer::root::reduce;
use nuvem::app::{Action, AppState, SystemAction};
use nuvem::config::problems::{ConfigProblemSource, ConfigProblems};
use nuvem::config::state;
use nuvem::services::runtime::session::RuntimeSession;
use nuvem::services::runtime::surface_startup_config_problems;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use serde::{Deserialize, Serialize};

use crate::support::frame_svg;

use frame_svg::buffer_to_svg;

const WIDTH: u16 = 100;
const HEIGHT: u16 = 28;

/// A stand-in for whatever a future feature persists. The writer is generic, so
/// the evidence uses a local payload exactly as a consumer would.
#[derive(Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Remembered {
    last_view: String,
    favorites: Vec<String>,
}

/// A config file with the things no serializer round-trips: a comment, a blank
/// line, and keys in the user's order rather than the struct's.
const HAND_WRITTEN_CONFIG: &str = "\
# my nuvem config -- hand edited, do not reformat
version = 1

[hotkeys]
help = \"H\"   # I use ? for search
";

fn evidence_dir() -> Option<PathBuf> {
    let dir = std::env::var("NUVEM_EVIDENCE_DIR").ok()?;
    let path = PathBuf::from(dir);
    fs::create_dir_all(&path).ok()?;
    Some(path)
}

fn write_text(name: &str, body: &str) {
    let Some(dir) = evidence_dir() else {
        eprintln!("NUVEM_EVIDENCE_DIR not set; skipping {name}");
        return;
    };
    fs::write(dir.join(name), body).expect("write evidence text");
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
        eprintln!("NUVEM_EVIDENCE_DIR not set; skipping frame {name}");
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

/// Decision 1: state lands in its own file beside the config, and the config
/// the user typed is not touched.
#[test]
fn state_is_written_beside_the_config_without_rewriting_it() {
    let dir = tempfile::tempdir().expect("temp dir");
    let config_path = dir.path().join("config.toml");
    fs::write(&config_path, HAND_WRITTEN_CONFIG).expect("write config");

    let state_path = state::state_path_for(&config_path);
    let remembered = Remembered {
        last_view: "compute".to_string(),
        favorites: vec!["acme-prod".to_string(), "acme-staging".to_string()],
    };
    state::save(&state_path, &remembered).expect("save state");

    let config_after = fs::read_to_string(&config_path).expect("read config");
    let state_after = fs::read_to_string(&state_path).expect("read state");
    let reloaded: Option<Remembered> = state::load(&state_path).expect("load state");

    assert_eq!(
        config_after, HAND_WRITTEN_CONFIG,
        "the hand-written config must be byte-for-byte untouched"
    );
    assert_eq!(state_path, dir.path().join("state.toml"));
    assert_eq!(reloaded, Some(remembered));

    let listing = sorted_listing(dir.path());
    write_text(
        "01-separation.txt",
        &format!(
            "1. A save next to a hand-edited config.\n\n\
             $ ls <config dir>\n{listing}\n\n\
             $ cat config.toml      # unchanged: comments, blank line, key order intact\n\
             {config_after}\n\
             $ cat state.toml       # written by the app, versioned envelope\n\
             {state_after}\n\
             $ # reading it back yields the payload that was saved\n\
             {reloaded:?}\n"
        ),
    );
}

fn sorted_listing(dir: &std::path::Path) -> String {
    let mut names: Vec<String> = fs::read_dir(dir)
        .expect("read dir")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names.join("\n")
}

/// Decision 2: the destination is replaced by rename, so a reader mid-save sees
/// the old complete file or the new one, never a truncated one — and nothing is
/// left staged afterwards.
#[test]
fn a_save_replaces_the_destination_and_leaves_nothing_staged() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("state.toml");

    state::save(
        &path,
        &Remembered {
            last_view: "compute".to_string(),
            ..Remembered::default()
        },
    )
    .expect("first save");
    let first_inode = fs::metadata(&path).expect("stat").ino();
    let first_body = fs::read_to_string(&path).expect("read");

    state::save(
        &path,
        &Remembered {
            last_view: "storage".to_string(),
            favorites: vec!["acme-prod".to_string()],
        },
    )
    .expect("second save");
    let second_inode = fs::metadata(&path).expect("stat").ino();
    let second_body = fs::read_to_string(&path).expect("read");

    assert_ne!(
        first_inode, second_inode,
        "an in-place rewrite would leave a window where the file is truncated"
    );
    let listing = sorted_listing(dir.path());
    assert_eq!(listing, "state.toml", "no staging file may survive a save");

    write_text(
        "02-atomic-replace.txt",
        &format!(
            "2. Saving twice over the same destination.\n\n\
             $ cat state.toml   # after the first save\n{first_body}\n\
             $ stat -f %i state.toml\n{first_inode}\n\n\
             $ cat state.toml   # after the second save\n{second_body}\n\
             $ stat -f %i state.toml\n{second_inode}\n\n\
             The inode changed, so the second save published a fully-written \
             file by rename\ninstead of truncating the one a concurrent reader \
             may be holding open.\n\n\
             $ ls\n{listing}\n\
             No `state.toml.tmp.<pid>.<n>` left behind.\n"
        ),
    );
}

/// Decisions 3 and 4: every way state can fail to load or save reaches the user
/// as a warning on the surface they already read config problems on.
#[test]
fn state_failures_reach_the_user_through_the_config_problems_overlay() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut problems = ConfigProblems::default();

    // Corrupt file.
    let corrupt = dir.path().join("corrupt.toml");
    fs::write(&corrupt, "this is : not = toml [[[").expect("write corrupt");
    let read: Remembered = state::load_or_default(&corrupt, &mut problems);
    assert_eq!(read, Remembered::default(), "a corrupt file is not adopted");

    // Written by a newer nuvem, and deliberately parseable as this payload.
    let future = dir.path().join("future.toml");
    fs::write(
        &future,
        "version = 99\n[data]\nlast_view = \"compute\"\nfavorites = []\n",
    )
    .expect("write future");
    let read: Remembered = state::load_or_default(&future, &mut problems);
    assert_eq!(
        read,
        Remembered::default(),
        "a newer version must not be adopted"
    );

    // Unwritable directory.
    let locked = dir.path().join("locked");
    fs::create_dir(&locked).expect("create locked dir");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o500)).expect("lock dir");
    state::save_reporting_problems(
        &locked.join("state.toml"),
        &Remembered::default(),
        &mut problems,
    );
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o700)).expect("unlock dir");

    assert_eq!(
        problems.iter().map(|p| p.source).collect::<Vec<_>>(),
        vec![
            ConfigProblemSource::State,
            ConfigProblemSource::State,
            ConfigProblemSource::State
        ],
        "all three paths must degrade loudly through problems.rs"
    );

    let messages: Vec<String> = problems.messages().iter().map(|m| m.to_string()).collect();

    let hotkeys = nuvem::config::hotkeys::KeyOverrides::default();
    let mut session = RuntimeSession::new_for_test(AppState::new());
    surface_startup_config_problems(&mut session, &problems, None, &hotkeys);
    let app = session.app().clone();
    let status = app
        .root
        .system
        .status_message
        .clone()
        .unwrap_or_else(|| "<no status message>".to_string());

    let startup = draw(&session);
    write_frame(
        "03-status-line",
        "State problems on the status line",
        "Three state failures (corrupt file, newer version, unwritable \
         directory) surfaced at startup. Nothing is swallowed and nothing \
         panics.",
        &startup,
    );

    let mut help_app = app;
    reduce(&mut help_app, &Action::System(SystemAction::ToggleHelp));
    let help_session = RuntimeSession::new_for_test(help_app);
    let overlay = draw(&help_session);
    let overlay_text = text(&overlay);
    assert!(
        overlay_text.contains("Config problems"),
        "the overlay must list the state problems:\n{overlay_text}"
    );
    for fragment in ["corrupt.toml", "future.toml", "99"] {
        assert!(
            overlay_text.contains(fragment),
            "the overlay should name {fragment:?}:\n{overlay_text}"
        );
    }
    write_frame(
        "04-help-overlay",
        "The same problems in the help overlay",
        "Each state warning names the file and says the app started from \
         defaults, so state that silently never loads or saves is \
         distinguishable from a feature that is merely broken.",
        &overlay,
    );

    write_text(
        "05-messages.txt",
        &format!(
            "3. The composed warnings, verbatim.\n\n\
             status line: {status}\n\n{}\n",
            messages
                .iter()
                .map(|m| format!("  - {m}"))
                .collect::<Vec<_>>()
                .join("\n")
        ),
    );
}

fn draw(session: &RuntimeSession) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(WIDTH, HEIGHT)).expect("terminal init");
    terminal
        .draw(|frame| nuvem::ui::render(frame, session))
        .expect("render");
    terminal.backend().buffer().clone()
}
