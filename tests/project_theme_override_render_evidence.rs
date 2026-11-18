//! Evidence: a `[theme.project_overrides]` entry reaches a painted
//! frame for a configuration it does not name, and a project no configuration
//! targets is reported rather than silently doing nothing.
//!
//! The problem in one sentence: the original guardrail keys on the gcloud
//! *configuration name*, so an operator whose prod is reached through a
//! configuration called `default` gets no red frame and no warning — nothing
//! is wrong, they simply declared no override for `default`. These assertions
//! are on painted cells for the same reason the original's are: an override that
//! parses but never reaches the frame passes every config test and fails in
//! the app.

use nuvem::app::{AppState, EffectResult};
use nuvem::config::Config;
use nuvem::config::theme::{Theme, ThemeChoice, ThemeOverrides};
use nuvem::services::runtime::session::RuntimeSession;
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use ratatui::style::Color;

use crate::support::theme::{painted_border_fg, theme_lock};

/// A configuration whose name says nothing about the project it points at —
/// the case the whole ticket is about.
fn configuration(name: &str, project: Option<&str>, is_active: bool) -> GcloudConfiguration {
    GcloudConfiguration {
        name: GcloudConfigurationName::new(name),
        is_active,
        account: Some("dev@example.com".to_string()),
        project: project.map(str::to_string),
        region: None,
        zone: None,
    }
}

fn config_from(raw: &str) -> Config {
    let document: toml::Value = toml::from_str(raw).expect("valid fixture TOML");
    let mut problems = nuvem::config::problems::ConfigProblems::default();
    let overrides = ThemeOverrides::from_file(
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

/// The load-bearing one.
#[tokio::test]
async fn a_project_override_repaints_the_frame_of_a_configuration_it_never_names() {
    let _guard = theme_lock().await;
    nuvem::config::theme::set_active(Theme::default_theme());

    let mut session = session_with(config_from(
        r#"
        [theme.project_overrides]
        acme-prod = "red"
        "#,
    ));

    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![configuration("default", Some("acme-prod"), true)],
        })
        .await;

    assert_eq!(
        painted_border_fg(false),
        Color::Red,
        "a configuration named `default` pointing at a prod project must be \
         unmistakable, or the guardrail is only as good as a naming convention"
    );
    assert_eq!(painted_border_fg(true), Color::Red);
}

/// The original's worst failure, on the new key: an override that looks configured
/// and quietly guards nothing. The configuration-name check cannot catch this
/// one, because the id it names is not a configuration name at all.
#[tokio::test]
async fn a_project_override_no_configuration_targets_warns_once_they_load() {
    let _guard = theme_lock().await;
    nuvem::config::theme::set_active(Theme::default_theme());

    let mut session = session_with(config_from(
        r#"
        [theme.project_overrides]
        acme-prd = "red"
        "#,
    ));

    assert!(
        session.app().root.system.config_problems().is_empty(),
        "the check must not fire before configurations are known"
    );

    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![configuration("default", Some("acme-prod"), true)],
        })
        .await;

    let problems = session.app().root.system.config_problems();
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(
        problems[0].contains("acme-prd"),
        "the warning must name the id the user has to fix: {:?}",
        problems[0]
    );
    assert!(
        problems[0].contains("project_overrides"),
        "and the table it is written in: {:?}",
        problems[0]
    );
}

#[tokio::test]
async fn a_project_override_some_configuration_targets_is_silent() {
    let _guard = theme_lock().await;
    nuvem::config::theme::set_active(Theme::default_theme());

    let mut session = session_with(config_from(
        r#"
        [theme.project_overrides]
        acme-prod = "red"
        "#,
    ));

    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![
                configuration("default", Some("acme-dev"), true),
                configuration("other", Some("acme-prod"), false),
            ],
        })
        .await;

    assert!(
        session.app().root.system.config_problems().is_empty(),
        "an override for a project some configuration reaches is not a typo: {:?}",
        session.app().root.system.config_problems()
    );
}

/// The twin of the original's "switching away restores the base frame", on the
/// project key: a guardrail that stays red after leaving the project is as
/// misleading as one that never fired.
#[tokio::test]
async fn leaving_an_overridden_project_restores_the_base_frame() {
    let _guard = theme_lock().await;
    nuvem::config::theme::set_active(Theme::default_theme());

    let mut session = session_with(config_from(
        r#"
        [theme.project_overrides]
        acme-prod = "red"
        "#,
    ));

    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![configuration("default", Some("acme-prod"), true)],
        })
        .await;
    assert_eq!(painted_border_fg(false), Color::Red);

    session
        .apply_effect_result(EffectResult::ConfigurationActivated {
            configuration: configuration("default", Some("acme-dev"), true),
        })
        .await;

    assert_eq!(
        painted_border_fg(false),
        Color::DarkGray,
        "the frame must follow the project the configuration now points at"
    );
}

/// A configuration with no project set has nothing to match, and must not
/// borrow another project's colour.
#[tokio::test]
async fn a_configuration_with_no_project_takes_no_project_override() {
    let _guard = theme_lock().await;
    nuvem::config::theme::set_active(Theme::default_theme());

    let mut session = session_with(config_from(
        r#"
        [theme.project_overrides]
        acme-prod = "red"
        "#,
    ));

    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![configuration("default", None, true)],
        })
        .await;

    assert_eq!(painted_border_fg(false), Color::DarkGray);
}

/// The mirror of the recompute guard the frame half already widened: the
/// deferred check reads projects, so its recompute key has to watch them too.
/// A project edited under a stable configuration name changes the answer
/// without changing any name.
#[tokio::test]
async fn a_project_edited_under_a_stable_name_re_runs_the_deferred_check() {
    let _guard = theme_lock().await;
    nuvem::config::theme::set_active(Theme::default_theme());

    let mut session = session_with(config_from(
        r#"
        [theme.project_overrides]
        acme-prod = "red"
        "#,
    ));

    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![configuration("default", Some("acme-dev"), true)],
        })
        .await;
    assert_eq!(
        session.app().root.system.config_problems().len(),
        1,
        "nothing targets acme-prod yet, so the override is reported"
    );

    // The same configuration, repointed at the overridden project.
    session
        .apply_effect_result(EffectResult::ConfigurationActivated {
            configuration: configuration("default", Some("acme-prod"), true),
        })
        .await;

    assert!(
        session.app().root.system.config_problems().is_empty(),
        "the override now applies, so the warning claiming it never will is \
         stale: {:?}",
        session.app().root.system.config_problems()
    );
}

/// The direction that matters more: an override that stops applying must
/// start warning, or the operator keeps a red-framed belief about a project
/// nothing reaches any more.
#[tokio::test]
async fn a_project_repointed_away_starts_warning_without_any_name_changing() {
    let _guard = theme_lock().await;
    nuvem::config::theme::set_active(Theme::default_theme());

    let mut session = session_with(config_from(
        r#"
        [theme.project_overrides]
        acme-prod = "red"
        "#,
    ));

    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![configuration("default", Some("acme-prod"), true)],
        })
        .await;
    assert!(session.app().root.system.config_problems().is_empty());

    session
        .apply_effect_result(EffectResult::ConfigurationActivated {
            configuration: configuration("default", Some("acme-dev"), true),
        })
        .await;

    let problems = session.app().root.system.config_problems();
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(problems[0].contains("acme-prod"), "{:?}", problems[0]);
}
