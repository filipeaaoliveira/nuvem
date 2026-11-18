//! The reloaded theme is applied through a process-global arc-swap
//! (`nuvem::config::theme`). These assertions need to be alone in their
//! process: any concurrent reload (which also calls `set_active`) races them.
//! That isolation used to come from this file being its own test binary; the
//! consolidated suite target shares one process, so the tiers and CI run the
//! suite under nextest, which runs each test in a process of its own. Both
//! tiers require nextest and refuse without it, so no gating path
//! can reach the shared-process run. Under a bare `cargo test` the race is
//! back — a rare flake here or in any theme-sensitive render test means
//! exactly that.

use std::io::Write;

use clap::Parser;
use nuvem::app::{Action, AppState, Effect};
use nuvem::config::theme::{self, Theme, ThemeChoice};
use nuvem::services::runtime::session::RuntimeSession;
use nuvem::{Cli, Config};

// The two scenarios below both mutate the process-global active theme, so they
// run as ONE sequential test (not two parallel `#[tokio::test]`s in the same
// binary): each resets the global to `default` before reloading to
// `high-contrast`, and parallel execution could let one reset clobber the
// other's assertion.
#[tokio::test]
async fn reload_applies_theme_live_including_under_field_level_warnings() {
    reload_applies_theme_live().await;
    reload_with_bad_alias_warning_still_applies_theme().await;
}

async fn reload_applies_theme_live() {
    let mut file = tempfile::NamedTempFile::new().expect("create temp config");
    file.write_all(b"version = 1\n[general]\ntheme = \"default\"\n")
        .expect("write temp config");
    file.flush().expect("flush temp config");

    let cli = Cli::parse_from([
        "nuvem",
        "--plugins",
        "/nonexistent/nuvem-plugins-test.toml",
        "--config",
        file.path().to_str().unwrap(),
    ]);
    let config = Config::resolve(&cli).config;
    assert_eq!(config.theme, ThemeChoice::Default);

    theme::set_active(Theme::default_theme());

    let mut app = AppState::new();
    app.root.aliases = config.aliases.clone();
    let mut session = RuntimeSession::new_with_cli_and_config_for_test(cli, config, app);

    std::fs::write(
        file.path(),
        "version = 1\n[general]\ntheme = \"high-contrast\"\n",
    )
    .expect("rewrite temp config");

    session
        .dispatch_action(Action::Effect(Effect::ReloadConfig))
        .await;

    let active = theme::active();
    assert_eq!(**active, Theme::high_contrast());
}

/// A reloaded file that is valid TOML with a correct version but has one
/// bad alias is a field-level warning (`load_error = None`), not a load
/// failure. `resolve` already substitutes a safe default for the bad alias,
/// so the rest of the reload — including the theme change — must still be
/// applied live, exactly like at startup. The reload must not be rejected
/// wholesale just because `warnings` is non-empty.
async fn reload_with_bad_alias_warning_still_applies_theme() {
    let mut file = tempfile::NamedTempFile::new().expect("create temp config");
    file.write_all(b"version = 1\n[general]\ntheme = \"default\"\n")
        .expect("write temp config");
    file.flush().expect("flush temp config");

    let cli = Cli::parse_from([
        "nuvem",
        "--plugins",
        "/nonexistent/nuvem-plugins-test.toml",
        "--config",
        file.path().to_str().unwrap(),
    ]);
    let config = Config::resolve(&cli).config;
    assert_eq!(config.theme, ThemeChoice::Default);

    theme::set_active(Theme::default_theme());

    let mut app = AppState::new();
    app.root.aliases = config.aliases.clone();
    let mut session = RuntimeSession::new_with_cli_and_config_for_test(cli, config, app);

    std::fs::write(
        file.path(),
        "version=1\n[aliases]\nfoo=\"not-a-real-service\"\n[general]\ntheme=\"high-contrast\"\n",
    )
    .expect("rewrite temp config");

    session
        .dispatch_action(Action::Effect(Effect::ReloadConfig))
        .await;

    // Field-level warning did not block the reload: the safe theme change
    // still applied live.
    let active = theme::active();
    assert_eq!(**active, Theme::high_contrast());

    // And the warning is surfaced in the status message rather than silently
    // dropped or treated as a rejection.
    let status = session
        .app()
        .root
        .system
        .status_message
        .clone()
        .unwrap_or_default();
    assert!(
        status.contains("warning"),
        "expected status to surface the field-level warning, got: {status:?}"
    );
    assert!(
        !status.contains("rejected") && !status.contains("failed"),
        "field-level warning must not be reported as a reload failure: {status:?}"
    );
}
