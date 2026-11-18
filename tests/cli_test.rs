use clap::Parser;
use std::path::PathBuf;

use nuvem::{Cli, Config};

#[test]
fn cli_accepts_configuration_and_log_file() {
    let cli = Cli::parse_from([
        "nuvem",
        "--configuration",
        "dev",
        "--log-file",
        "/tmp/nuvem.log",
        "--poll-interval",
        "10",
    ]);

    let config = Config::from_cli(&cli);

    assert_eq!(config.starting_configuration.as_deref(), Some("dev"));
    assert_eq!(config.log_file, PathBuf::from("/tmp/nuvem.log"));
    assert_eq!(config.poll_interval_secs, 10);
}

#[test]
fn cli_defaults_poll_interval_when_flag_omitted() {
    let cli = Cli::parse_from(["nuvem", "--configuration", "dev"]);

    let config = Config::from_cli(&cli);

    assert_eq!(
        config.poll_interval_secs,
        Config::default().poll_interval_secs
    );
}
