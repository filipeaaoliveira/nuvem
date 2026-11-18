use clap::Parser;
use nuvem::{Cli, config::run_info};

#[test]
fn info_reports_paths_without_starting_tui() {
    let cli = Cli::parse_from([
        "nuvem",
        "info",
        "--plugins",
        "/nonexistent/nuvem-plugins-test.toml",
        "--config",
        "/nonexistent/nuvem.toml",
    ]);
    let out = run_info(&cli);
    assert!(out.contains("config:"));
    assert!(out.contains("/nonexistent/nuvem.toml"));
    assert!(out.contains("not found"));
    assert!(out.contains("log:"));
    assert!(out.contains("cache:"));
    assert!(out.contains("theme:"));
}

#[test]
fn info_reports_error_status_for_bad_file() {
    let mut f = tempfile::NamedTempFile::new().unwrap();
    std::io::Write::write_all(&mut f, b"[[[bad").unwrap();
    let cli = Cli::parse_from([
        "nuvem",
        "info",
        "--plugins",
        "/nonexistent/nuvem-plugins-test.toml",
        "--config",
        f.path().to_str().unwrap(),
    ]);
    let out = run_info(&cli);
    assert!(out.contains("error:"));
}

#[test]
fn info_reports_loaded_status_with_warning_for_bad_alias() {
    let mut f = tempfile::NamedTempFile::new().unwrap();
    std::io::Write::write_all(
        &mut f,
        b"version = 1\n[aliases]\nfoo = \"not-a-real-service\"\n",
    )
    .unwrap();
    let cli = Cli::parse_from([
        "nuvem",
        "info",
        "--plugins",
        "/nonexistent/nuvem-plugins-test.toml",
        "--config",
        f.path().to_str().unwrap(),
    ]);
    let out = run_info(&cli);
    assert!(out.contains("(loaded)"));
    assert!(!out.contains("error:"));
    assert!(out.contains("warning:"));
    assert!(out.contains("foo"));
}

#[test]
fn info_reports_loaded_status_with_no_warnings_for_valid_file() {
    let mut f = tempfile::NamedTempFile::new().unwrap();
    std::io::Write::write_all(&mut f, b"version = 1\n").unwrap();
    let cli = Cli::parse_from([
        "nuvem",
        "info",
        "--plugins",
        "/nonexistent/nuvem-plugins-test.toml",
        "--config",
        f.path().to_str().unwrap(),
    ]);
    let out = run_info(&cli);
    assert!(out.contains("(loaded)"));
    assert!(!out.contains("error:"));
    assert!(!out.contains("warning:"));
}
