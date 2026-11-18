//! Evidence: a configured frame override actually reaches a painted
//! frame, and an override naming no real configuration is reported.
//!
//! Its own test binary on purpose. Both the active theme
//! (`config::theme::set_active`) and the frame it paints are process-wide, so
//! a test here running beside the parser tests in another binary would race
//! them — the same reason `config_reload_theme_test.rs` stands alone.
//!
//! Within this binary the ordering is enforced rather than assumed: every
//! test in this file and in the project-override evidence beside it takes `theme_lock`,
//! because `cargo test` runs a binary's tests on concurrent threads by
//! default. That lock is shared through `support::theme` so the two files
//! serialize against each other rather than each only against itself; see
//! that module for what it does and does not cover.

use clap::Parser;

use nuvem::Cli;
use nuvem::app::{Action, AppState, Effect, EffectResult};
use nuvem::config::Config;
use nuvem::config::hotkeys::KeyOverrides;
use nuvem::config::problems::ConfigProblems;
use nuvem::config::theme::{Theme, ThemeChoice};
use nuvem::services::runtime::session::RuntimeSession;
use nuvem::services::runtime::surface_startup_config_problems;
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};

use crate::support::theme::{painted_border_fg, theme_lock};
use ratatui::style::Color;

fn configuration(name: &str, is_active: bool) -> GcloudConfiguration {
    GcloudConfiguration {
        name: GcloudConfigurationName::new(name),
        is_active,
        account: Some("dev@example.com".to_string()),
        project: Some("a-project".to_string()),
        region: None,
        zone: None,
    }
}

fn config_with_overrides(pairs: &[(&str, &str)]) -> Config {
    let table = pairs
        .iter()
        .map(|(name, color)| format!("{name} = \"{color}\""))
        .collect::<Vec<_>>()
        .join("\n");
    let document: toml::Value =
        toml::from_str(&format!("[theme.overrides]\n{table}\n")).expect("valid fixture TOML");

    let mut problems = nuvem::config::problems::ConfigProblems::default();
    let overrides = nuvem::config::theme::ThemeOverrides::from_file(
        document.get("theme").expect("fixture has [theme]"),
        &mut problems,
    );
    assert!(
        problems.into_messages().is_empty(),
        "fixture overrides must parse cleanly"
    );

    Config {
        theme: ThemeChoice::Default,
        theme_overrides: overrides,
        ..Config::default()
    }
}

fn session_with(config: Config) -> RuntimeSession {
    RuntimeSession::new_with_config_for_test(config, AppState::new())
}

/// The load-bearing one. A theme override that is configured but never
/// reaches the frame passes every config test and fails in the app, so this
/// asserts on painted cells rather than on the `Theme` struct.
#[tokio::test]
async fn activating_an_overridden_configuration_repaints_the_frame() {
    let _guard = theme_lock().await;
    nuvem::config::theme::set_active(Theme::default_theme());

    let mut session = session_with(config_with_overrides(&[("prod", "red")]));

    // Before any configuration is known, the frame is the base theme's.
    assert_eq!(
        painted_border_fg(false),
        Color::DarkGray,
        "an unconfigured frame must be untouched"
    );

    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![configuration("prod", true)],
        })
        .await;

    assert_eq!(
        painted_border_fg(false),
        Color::Red,
        "the override must reach the painted unfocused border"
    );
    assert_eq!(
        painted_border_fg(true),
        Color::Red,
        "the override must reach the painted focused border too"
    );
}

#[tokio::test]
async fn switching_to_a_configuration_without_an_override_restores_the_base_frame() {
    let _guard = theme_lock().await;
    nuvem::config::theme::set_active(Theme::default_theme());

    let mut session = session_with(config_with_overrides(&[("prod", "red")]));

    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![configuration("prod", true), configuration("dev", false)],
        })
        .await;
    assert_eq!(painted_border_fg(false), Color::Red);

    session
        .apply_effect_result(EffectResult::ConfigurationActivated {
            configuration: configuration("dev", true),
        })
        .await;

    assert_eq!(
        painted_border_fg(false),
        Color::DarkGray,
        "leaving an overridden configuration must restore the base frame, or \
         the guardrail would stay red on a configuration that is not prod"
    );
}

#[tokio::test]
async fn a_configuration_with_no_override_never_repaints_the_frame() {
    let _guard = theme_lock().await;
    nuvem::config::theme::set_active(Theme::default_theme());

    let mut session = session_with(config_with_overrides(&[("prod", "red")]));
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![configuration("dev", true)],
        })
        .await;

    assert_eq!(painted_border_fg(false), Color::DarkGray);
}

// --- the deferred "names nothing" check -----------------------------------

#[tokio::test]
async fn an_override_naming_no_real_configuration_warns_once_they_load() {
    let _guard = theme_lock().await;
    nuvem::config::theme::set_active(Theme::default_theme());

    let mut session = session_with(config_with_overrides(&[("prd", "red")]));

    // Nothing to check against yet, so nothing is claimed.
    assert!(
        session.app().root.system.config_problems().is_empty(),
        "the check must not fire before configurations are known"
    );

    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![configuration("prod", true)],
        })
        .await;

    let problems = session.app().root.system.config_problems();
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(
        problems[0].contains("prd"),
        "the warning must name the misspelling the user has to fix: {:?}",
        problems[0]
    );
}

#[tokio::test]
async fn an_override_naming_a_real_configuration_is_silent() {
    let _guard = theme_lock().await;
    nuvem::config::theme::set_active(Theme::default_theme());

    let mut session = session_with(config_with_overrides(&[("prod", "red")]));
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![configuration("prod", true)],
        })
        .await;

    assert!(
        session.app().root.system.config_problems().is_empty(),
        "a valid override must not warn: {:?}",
        session.app().root.system.config_problems()
    );
}

/// Configurations load more than once (a reload, an activation that refreshes
/// the list). The deferred check rebuilds the list wholesale rather than
/// appending, so a repeat load must not stack a second copy of the warning.
#[tokio::test]
async fn the_deferred_warning_is_not_duplicated_by_a_repeat_load() {
    let _guard = theme_lock().await;
    nuvem::config::theme::set_active(Theme::default_theme());

    let mut session = session_with(config_with_overrides(&[("prd", "red")]));
    for _ in 0..3 {
        session
            .apply_effect_result(EffectResult::ConfigurationsLoaded {
                configurations: vec![configuration("prod", true)],
            })
            .await;
    }

    assert_eq!(
        session.app().root.system.config_problems().len(),
        1,
        "{:?}",
        session.app().root.system.config_problems()
    );
}

/// The deferred check adds to the resolve-time problems; it must not replace
/// them, or a bad alias would vanish from the help overlay the moment
/// configurations loaded.
#[tokio::test]
async fn the_deferred_check_preserves_the_resolve_time_problems() {
    let _guard = theme_lock().await;
    nuvem::config::theme::set_active(Theme::default_theme());

    let mut session = session_with(config_with_overrides(&[("prd", "red")]));
    session.set_base_config_problems(vec!["unknown general.default_service".to_string()]);

    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![configuration("prod", true)],
        })
        .await;

    let problems = session.app().root.system.config_problems();
    assert_eq!(problems.len(), 2, "{problems:?}");
    assert!(problems[0].contains("default_service"), "{problems:?}");
    assert!(problems[1].contains("prd"), "{problems:?}");
}

// --- the deferred warning on the status line -------------------------------
//
// The help overlay is not enough on its own: a user who typo'd `prod` has no
// reason to press the help key, so the status line is the only surface that
// reaches them proactively.

fn status_of(session: &RuntimeSession) -> String {
    session
        .app()
        .root
        .system
        .status_message
        .clone()
        .expect("a status message is set")
}

#[tokio::test]
async fn the_deferred_warning_reaches_the_startup_status_line() {
    let _guard = theme_lock().await;
    nuvem::config::theme::set_active(Theme::default_theme());

    let mut session = session_with(config_with_overrides(&[("prd", "red")]));
    // What `initialize()` does at startup: the configurations load, which is
    // what makes the deferred check answerable at all.
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![configuration("prod", true)],
        })
        .await;

    // The rest of the startup sequence, over an otherwise-clean resolve whose
    // only problem is the deferred one.
    surface_startup_config_problems(
        &mut session,
        &ConfigProblems::default(),
        None,
        &KeyOverrides::empty(),
    );

    let status = status_of(&session);
    assert!(
        status.contains("prd"),
        "the typo must be announced, not left to the help overlay: {status:?}"
    );
}

#[tokio::test]
async fn the_deferred_warning_reaches_the_reload_status_line() {
    let _guard = theme_lock().await;
    nuvem::config::theme::set_active(Theme::default_theme());

    let file = tempfile::NamedTempFile::new().expect("create temp config");
    std::fs::write(file.path(), "[theme.overrides]\nprd = \"red\"\n").expect("write temp config");
    let cli = Cli::parse_from([
        "nuvem",
        "--plugins",
        "/nonexistent/nuvem-plugins-test.toml",
        "--config",
        file.path().to_str().expect("utf-8 path"),
    ]);

    let mut session =
        RuntimeSession::new_with_cli_and_config_for_test(cli, Config::default(), AppState::new());
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![configuration("prod", true)],
        })
        .await;

    session
        .dispatch_action(Action::Effect(Effect::ReloadConfig))
        .await;

    let status = status_of(&session);
    assert!(
        status.contains("prd"),
        "a reload whose only problem is an unknown override name must say so: {status:?}"
    );
}
