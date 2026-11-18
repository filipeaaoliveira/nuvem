//! Per-configuration theme overrides: the `[theme.overrides]`
//! parser and the `Theme` recolor it feeds.
//!
//! Nothing here touches the process-wide active theme. The one test that
//! does — the rendering assertion — lives in its own binary
//! (`theme_override_render_evidence.rs`), because `set_active` is a
//! process global and tests in a binary run concurrently.

use nuvem::config::problems::{ConfigProblemSource, ConfigProblems};
use nuvem::config::theme::{Theme, ThemeChoice, ThemeOverrides};
use ratatui::style::{Color, Modifier};

fn parse(raw: &str) -> (ThemeOverrides, Vec<String>) {
    let value: toml::Value = toml::from_str(raw).expect("test fixture is valid TOML");
    let theme = value
        .get("theme")
        .expect("test fixture has a [theme] table")
        .clone();
    let mut problems = ConfigProblems::default();
    let overrides = ThemeOverrides::from_file(&theme, &mut problems);
    (overrides, problems.into_messages())
}

#[test]
fn an_override_parses_to_the_color_it_names() {
    let (overrides, problems) = parse(
        r#"
        [theme.overrides]
        prod = "red"
        "#,
    );

    assert!(problems.is_empty(), "clean config warned: {problems:?}");
    assert_eq!(overrides.border_color_for("prod", None), Some(Color::Red));
}

#[test]
fn a_configuration_with_no_override_gets_none() {
    let (overrides, _) = parse(
        r#"
        [theme.overrides]
        prod = "red"
        "#,
    );

    assert_eq!(overrides.border_color_for("staging", None), None);
}

/// The lenient spellings ratatui's own `Color: FromStr` accepts. Pinned so a
/// future hand-rolled parser cannot quietly narrow what a user may write.
#[test]
fn color_spellings_are_lenient() {
    for (raw, expected) in [
        ("red", Color::Red),
        ("Red", Color::Red),
        ("light-red", Color::LightRed),
        ("light_red", Color::LightRed),
        ("#ff0000", Color::Rgb(255, 0, 0)),
        ("9", Color::Indexed(9)),
    ] {
        let (overrides, problems) = parse(&format!(
            r#"
            [theme.overrides]
            prod = "{raw}"
            "#
        ));
        assert!(problems.is_empty(), "{raw:?} warned: {problems:?}");
        assert_eq!(
            overrides.border_color_for("prod", None),
            Some(expected),
            "{raw:?}"
        );
    }
}

#[test]
fn an_unparseable_color_is_dropped_with_a_warning_and_siblings_survive() {
    let (overrides, problems) = parse(
        r#"
        [theme.overrides]
        prod = "not-a-color"
        staging = "yellow"
        "#,
    );

    assert_eq!(
        overrides.border_color_for("prod", None),
        None,
        "an unparseable color must not reach the frame"
    );
    assert_eq!(
        overrides.border_color_for("staging", None),
        Some(Color::Yellow),
        "one bad entry must not cost a valid sibling its override"
    );
    assert_eq!(
        problems.len(),
        1,
        "expected exactly one warning: {problems:?}"
    );
    assert!(
        problems[0].contains("prod") && problems[0].contains("not-a-color"),
        "the warning must name the key and the value the user has to fix: {:?}",
        problems[0]
    );
}

#[test]
fn a_non_string_value_is_dropped_with_a_warning() {
    let (overrides, problems) = parse(
        r#"
        [theme.overrides]
        prod = 5
        staging = "yellow"
        "#,
    );

    assert_eq!(overrides.border_color_for("prod", None), None);
    assert_eq!(
        overrides.border_color_for("staging", None),
        Some(Color::Yellow)
    );
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(problems[0].contains("prod"), "{:?}", problems[0]);
}

#[test]
fn a_blank_configuration_name_is_dropped_with_a_warning() {
    let (overrides, problems) = parse(
        r#"
        [theme.overrides]
        "   " = "red"
        "#,
    );

    assert!(overrides.is_empty());
    assert_eq!(problems.len(), 1, "{problems:?}");
}

#[test]
fn a_non_table_theme_section_is_dropped_whole_with_one_warning() {
    let value: toml::Value = toml::from_str("theme = \"red\"").unwrap();
    let mut problems = ConfigProblems::default();
    let overrides = ThemeOverrides::from_file(value.get("theme").unwrap(), &mut problems);

    assert!(overrides.is_empty());
    assert_eq!(problems.len(), 1, "{problems:?}");
    // Writing `theme = "dark"` at top level is the likeliest confusion now
    // that a `[theme]` section exists, so this warning points at the key the
    // user wanted exactly as the unknown-key one does.
    assert!(
        problems.messages()[0].contains("general.theme"),
        "{problems:?}"
    );
}

/// `[general].theme` picks the base theme and `[theme]` overrides it. Writing
/// the base theme's key inside `[theme]` is the likeliest confusion between
/// them, so it has to warn rather than be silently ignored.
#[test]
fn an_unknown_key_in_the_theme_section_warns() {
    let (overrides, problems) = parse(
        r#"
        [theme]
        active = "high-contrast"
        "#,
    );

    assert!(overrides.is_empty());
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(problems[0].contains("active"), "{:?}", problems[0]);
    assert!(
        problems[0].contains("general.theme"),
        "the warning must point at the key the user actually wants rather than \
         only saying they are wrong: {:?}",
        problems[0]
    );
}

#[test]
fn theme_problems_are_tagged_as_theme_problems() {
    let value: toml::Value = toml::from_str(
        r#"
        [theme.overrides]
        prod = "not-a-color"
        "#,
    )
    .unwrap();
    let mut problems = ConfigProblems::default();
    ThemeOverrides::from_file(value.get("theme").unwrap(), &mut problems);

    assert_eq!(
        problems.iter().map(|p| p.source).collect::<Vec<_>>(),
        vec![ConfigProblemSource::Theme]
    );
}

/// Pins `is_empty` in the direction the other tests never exercise. They only
/// ever assert it is *true* (on a section that parsed to nothing), so an
/// implementation that always answered `true` would satisfy every one of them
/// while telling the deferred check there is nothing to validate.
#[test]
fn is_empty_is_false_once_an_override_parses() {
    let (overrides, problems) = parse(
        r#"
        [theme.overrides]
        prod = "red"
        "#,
    );

    assert!(problems.is_empty(), "{problems:?}");
    assert!(
        !overrides.is_empty(),
        "a table with a valid override is not empty"
    );
}

#[test]
fn configurations_lists_every_overridden_name() {
    let (overrides, _) = parse(
        r#"
        [theme.overrides]
        prod = "red"
        staging = "yellow"
        "#,
    );

    assert_eq!(
        overrides.configurations().collect::<Vec<_>>(),
        vec!["prod", "staging"]
    );
}

// --- the recolor itself ---------------------------------------------------

#[test]
fn with_border_color_recolors_both_border_roles() {
    let themed = Theme::default_theme().with_border_color(Color::Red);

    assert_eq!(themed.border.fg, Some(Color::Red));
    assert_eq!(
        themed.focused_border.fg,
        Some(Color::Red),
        "a guardrail that only shows while unfocused reads as unreliable"
    );
}

#[test]
fn with_border_color_leaves_every_other_role_alone() {
    let base = Theme::default_theme();
    let themed = base.with_border_color(Color::Red);

    assert_eq!(themed.app_title, base.app_title);
    assert_eq!(themed.selected_row, base.selected_row);
    assert_eq!(themed.status_error, base.status_error);
    assert_eq!(themed.overlay_border, base.overlay_border);
    assert_eq!(themed.muted, base.muted);
}

/// The override replaces the hue, not the base theme's non-color emphasis:
/// `high-contrast` keeps its bold focus border and `no-color` keeps its
/// underline, so an override cannot silently undo an accessibility choice.
#[test]
fn with_border_color_preserves_modifiers() {
    let high_contrast = Theme::for_choice(ThemeChoice::HighContrast).with_border_color(Color::Red);
    assert!(
        high_contrast
            .focused_border
            .add_modifier
            .contains(Modifier::BOLD),
        "high-contrast must keep its bold focus border"
    );

    let no_color = Theme::for_choice(ThemeChoice::NoColor).with_border_color(Color::Red);
    assert!(
        no_color
            .focused_border
            .add_modifier
            .contains(Modifier::UNDERLINED),
        "no-color must keep its underlined focus marker"
    );
}
