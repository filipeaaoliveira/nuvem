use std::io::Write;

use clap::Parser;
use nuvem::{Cli, Config};

fn write_cfg(body: &str) -> tempfile::NamedTempFile {
    let mut f = tempfile::NamedTempFile::new().unwrap();
    f.write_all(body.as_bytes()).unwrap();
    f
}

#[test]
fn defaults_apply_with_no_file() {
    let cli = Cli::parse_from([
        "nuvem",
        "--plugins",
        "/nonexistent/nuvem-plugins-test.toml",
        "--config",
        "/nonexistent/nuvem.toml",
    ]);
    let out = Config::resolve(&cli);
    assert_eq!(out.config.poll_interval_secs, 5);
    assert!(out.config.sidebar_visible);
    assert!(out.problems.is_empty());
}

#[test]
fn file_overrides_defaults() {
    let f = write_cfg("version = 1\n[general]\npoll_interval_secs = 12\nsidebar_visible = false\n");
    let cli = Cli::parse_from([
        "nuvem",
        "--plugins",
        "/nonexistent/nuvem-plugins-test.toml",
        "--config",
        f.path().to_str().unwrap(),
    ]);
    let out = Config::resolve(&cli);
    assert_eq!(out.config.poll_interval_secs, 12);
    assert!(!out.config.sidebar_visible);
}

#[test]
fn cli_flag_overrides_file() {
    let f = write_cfg("version = 1\n[general]\npoll_interval_secs = 12\n");
    let cli = Cli::parse_from([
        "nuvem",
        "--plugins",
        "/nonexistent/nuvem-plugins-test.toml",
        "--config",
        f.path().to_str().unwrap(),
        "--poll-interval",
        "30",
    ]);
    let out = Config::resolve(&cli);
    assert_eq!(out.config.poll_interval_secs, 30);
}

#[test]
fn malformed_file_warns_and_defaults() {
    let f = write_cfg("this is : not = toml [[[");
    let cli = Cli::parse_from([
        "nuvem",
        "--plugins",
        "/nonexistent/nuvem-plugins-test.toml",
        "--config",
        f.path().to_str().unwrap(),
    ]);
    let out = Config::resolve(&cli);
    assert_eq!(out.config.poll_interval_secs, 5);
    assert!(
        out.problems
            .messages()
            .iter()
            .any(|w| w.contains(f.path().to_str().unwrap()))
    );
}

#[test]
fn unknown_version_is_rejected_with_warning() {
    let f = write_cfg("version = 999\n[general]\npoll_interval_secs = 9\n");
    let cli = Cli::parse_from([
        "nuvem",
        "--plugins",
        "/nonexistent/nuvem-plugins-test.toml",
        "--config",
        f.path().to_str().unwrap(),
    ]);
    let out = Config::resolve(&cli);
    assert_eq!(out.config.poll_interval_secs, 5); // rejected -> defaults
    assert!(
        out.problems
            .messages()
            .iter()
            .any(|w| w.to_lowercase().contains("version"))
    );
}

// --- Unknown keys are named, not swallowed ---

/// Resolves `body` and returns the warnings it produced.
fn problems_for(body: &str) -> Vec<String> {
    let file = write_cfg(body);
    let cli = Cli::parse_from([
        "nuvem",
        "--plugins",
        "/nonexistent/nuvem-plugins-test.toml",
        "--config",
        file.path().to_str().unwrap(),
    ]);
    Config::resolve(&cli)
        .problems
        .messages()
        .into_iter()
        .map(str::to_string)
        .collect()
}

/// The near miss that motivated this: one character from `poll_interval_secs`,
/// and until now the file loaded clean while the setting did nothing. The
/// operator's evidence was a poll interval that ignored them.
#[test]
fn a_misspelled_general_setting_is_named_rather_than_dropped() {
    let messages = problems_for("version = 1\n[general]\npoll_interval_sec = 30\n");

    let reported = messages.join("\n");
    assert!(
        reported.contains("poll_interval_sec"),
        "the operator must be told which key was ignored: {reported}"
    );
}

/// Warning, not failing: the whole file must still apply. `deny_unknown_fields`
/// would have discarded the theme along with the typo.
#[test]
fn the_rest_of_the_file_still_applies_around_an_unknown_key() {
    let file = write_cfg(
        "version = 1\n[general]\npoll_interval_sec = 30\npoll_interval_secs = 42\ntheme = \"high-contrast\"\n",
    );
    let cli = Cli::parse_from([
        "nuvem",
        "--plugins",
        "/nonexistent/nuvem-plugins-test.toml",
        "--config",
        file.path().to_str().unwrap(),
    ]);

    let out = Config::resolve(&cli);

    assert_eq!(
        out.config.poll_interval_secs, 42,
        "the real setting beside the typo must still take effect"
    );
    assert!(
        out.load_error.is_none(),
        "an unknown key must not fail the load: {:?}",
        out.load_error
    );
}

/// A stray table at the root lands in the same net — `[genera]` for
/// `[general]` is the misspelling that costs the most.
#[test]
fn an_unknown_top_level_table_is_named() {
    let messages = problems_for("version = 1\n[genera]\ntheme = \"high-contrast\"\n");

    assert!(messages.join("\n").contains("genera"), "{messages:?}");
}

/// And a clean file stays silent: a warning for a key the schema does name
/// would be worse than the silence it replaces.
#[test]
fn every_documented_setting_is_accepted_without_warning() {
    let messages = problems_for(
        "version = 1\n\
         [general]\n\
         poll_interval_secs = 10\n\
         default_service = \"compute\"\n\
         sidebar_visible = false\n\
         theme = \"high-contrast\"\n\
         read_only = true\n",
    );

    assert!(messages.is_empty(), "{messages:?}");
}
