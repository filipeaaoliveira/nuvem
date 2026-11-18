//! The one lock for the one process-global active theme
//! (`nuvem::config::theme`'s arc-swap).
//!
//! The frame-override evidence takes this guard, because
//! those tests store to the global and then assert on what a rendered frame
//! carries — the one pairing where another store landing in between changes
//! the answer. It lives in `support` rather than in a test file because a
//! per-file lock only serializes that file: two files each with their own
//! mutex, mounted into one suite binary, race each other exactly as if
//! neither had locked.
//!
//! It is NOT taken by every test that touches the global. `config_theme_test`,
//! `config_reload_theme_test` and `config_evidence` all call `set_active`
//! without it, so under a bare `cargo test` — one process for the whole suite
//! — a store from one of those can still land between a guarded test's store
//! and its read. That is the race `tests/suite.rs` and `.config/nextest.toml`
//! already document as the known cost of the consolidated binary, and this
//! lock narrows it rather than closing it.
//!
//! Under the tiers and CI the question does not arise: both require nextest,
//! which gives each test its own process.

use std::sync::OnceLock;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::Color;
use tokio::sync::{Mutex, MutexGuard};

use nuvem::ui::shared::components::{bordered_block, focused_block};

/// Serializes every test that reads or writes the process-wide active theme.
///
/// An async mutex rather than `std::sync`: callers hold the guard across
/// `apply_effect_result` awaits, and a blocking guard held across an await is
/// the shape that deadlocks a multi-threaded runtime.
pub(crate) async fn theme_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(())).lock().await
}

/// The border foreground the *painted* frame actually carries. Renders a real
/// pane block through a `TestBackend` and reads the top-left corner cell,
/// which is always a border glyph.
///
/// Asserting on painted cells rather than on the `Theme` struct is the point:
/// an override that is configured but never reaches the frame passes every
/// config test and fails in the app.
pub(crate) fn painted_border_fg(focused: bool) -> Color {
    let mut terminal = Terminal::new(TestBackend::new(20, 5)).unwrap();
    terminal
        .draw(|frame| {
            let block = if focused {
                focused_block("Pane")
            } else {
                bordered_block("Pane")
            };
            frame.render_widget(block, frame.area());
        })
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    buffer[(0, 0)].fg
}
