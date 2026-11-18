//! Per-project theme overrides: the `[theme.project_overrides]`
//! parser and the precedence between a project override and the
//! configuration-name overrides that shipped first.
//!
//! Nothing here touches the process-wide active theme, for the reason
//! `theme_overrides_test.rs` records: the evidence that a project
//! override reaches a painted frame lives in its own binary.

use nuvem::config::problems::ConfigProblems;
use nuvem::config::theme::ThemeOverrides;
use ratatui::style::Color;

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

/// The whole point of the ticket: an operator whose prod is reached through a
/// configuration named `default` gets the frame anyway, because the override
/// names the project rather than the local alias.
#[test]
fn a_project_override_colors_a_configuration_it_does_not_name() {
    let (overrides, problems) = parse(
        r#"
        [theme.project_overrides]
        acme-prod = "red"
        "#,
    );

    assert!(problems.is_empty(), "clean config warned: {problems:?}");
    assert_eq!(
        overrides.border_color_for("default", Some("acme-prod")),
        Some(Color::Red)
    );
}

/// Precedence, stated rather than left to fall out of iteration order: the
/// configuration name is the narrower key, since several configurations can
/// point at one project, so a per-configuration declaration beats the
/// project-wide one.
#[test]
fn a_configuration_override_beats_a_project_override_for_the_same_context() {
    let (overrides, problems) = parse(
        r#"
        [theme.overrides]
        default = "yellow"

        [theme.project_overrides]
        acme-prod = "red"
        "#,
    );

    assert!(problems.is_empty(), "clean config warned: {problems:?}");
    assert_eq!(
        overrides.border_color_for("default", Some("acme-prod")),
        Some(Color::Yellow)
    );
}

/// A file whose only overrides are project ones is not an empty override set.
/// `is_empty` answering `true` there would describe a configured guardrail as
/// absent.
#[test]
fn is_empty_is_false_for_a_project_only_override_set() {
    let (overrides, problems) = parse(
        r#"
        [theme.project_overrides]
        acme-prod = "red"
        "#,
    );

    assert!(problems.is_empty(), "{problems:?}");
    assert!(!overrides.is_empty());
}

/// The vocabulary the deferred "targets nothing" check reads, sorted for the
/// same reason `configurations()` is.
#[test]
fn projects_lists_every_overridden_project_id() {
    let (overrides, _) = parse(
        r#"
        [theme.project_overrides]
        acme-prod = "red"
        acme-dev = "green"
        "#,
    );

    assert_eq!(
        overrides.projects().collect::<Vec<_>>(),
        vec!["acme-dev", "acme-prod"]
    );
}

/// The two tables are two namespaces. A project id leaking into the
/// configuration vocabulary would make the deferred check warn that
/// every project id names no gcloud configuration — true, and useless.
#[test]
fn a_project_override_is_not_listed_as_a_configuration_name() {
    let (overrides, _) = parse(
        r#"
        [theme.overrides]
        prod = "red"

        [theme.project_overrides]
        acme-prod = "red"
        "#,
    );

    assert_eq!(overrides.configurations().collect::<Vec<_>>(), vec!["prod"]);
    assert_eq!(overrides.projects().collect::<Vec<_>>(), vec!["acme-prod"]);
}

/// The warnings have to name what the reader wrote. A project table that
/// reports a bad key as a "configuration name" sends them to the wrong table.
#[test]
fn a_blank_project_id_warns_as_a_project_id() {
    let (overrides, problems) = parse(
        r#"
        [theme.project_overrides]
        "   " = "red"
        "#,
    );

    assert!(overrides.is_empty());
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(
        problems[0].contains("project id"),
        "the warning must name the vocabulary of the table it came from: {:?}",
        problems[0]
    );
}

#[test]
fn a_non_table_project_overrides_section_warns_about_project_ids() {
    let (overrides, problems) = parse(
        r#"
        [theme]
        project_overrides = "red"
        "#,
    );

    assert!(overrides.is_empty());
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(problems[0].contains("project id"), "{:?}", problems[0]);
}

/// `[theme.overrides.projects]` is the natural guess — it is the spelling
/// once proposed — and it is not the one that exists, because a
/// configuration may legitimately be named `projects` and reserving the name
/// inside a table of user-chosen names would make that configuration
/// uncolourable. The guess has to be told where to go rather than told only
/// that a colour must be a string.
#[test]
fn the_nested_projects_spelling_is_redirected_to_the_real_table() {
    let (overrides, problems) = parse(
        r#"
        [theme.overrides.projects]
        acme-prod = "red"
        "#,
    );

    assert!(overrides.is_empty());
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(
        problems[0].contains("theme.project_overrides"),
        "the warning must name the table that works: {:?}",
        problems[0]
    );
}

/// A configuration really named `projects` keeps its override, which is the
/// whole reason the table is a sibling rather than nested.
#[test]
fn a_configuration_named_projects_can_still_be_overridden() {
    let (overrides, problems) = parse(
        r#"
        [theme.overrides]
        projects = "red"
        "#,
    );

    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(
        overrides.border_color_for("projects", None),
        Some(Color::Red)
    );
}
