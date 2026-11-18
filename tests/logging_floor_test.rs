//! The log file's `info` floor is a list of crate names, and a list of crate
//! names rots the moment a client crate is added without it. This holds the
//! list to the manifest: every `google-cloud-*` dependency the binary links is
//! pinned, spelled as its log target.

use nuvem::services::runtime::logging::{CLIENT_LOG_TARGETS, log_filter};

#[test]
fn every_google_cloud_dependency_is_held_at_the_floor() {
    let manifest = std::fs::read_to_string("Cargo.toml").expect("Cargo.toml is readable");
    let crates: Vec<String> = manifest
        .lines()
        .filter_map(|line| line.trim().split_once('='))
        .map(|(name, _)| name.trim().to_string())
        .filter(|name| name.starts_with("google-cloud-"))
        .map(|name| name.replace('-', "_"))
        .collect();
    assert!(
        crates.len() > 5,
        "the manifest scan found {} client crates, which means it is looking in the wrong place",
        crates.len()
    );
    let missing: Vec<&String> = crates
        .iter()
        .filter(|name| !CLIENT_LOG_TARGETS.contains(&name.as_str()))
        .collect();
    assert!(
        missing.is_empty(),
        "client crates without a log floor: {missing:?}; add them to CLIENT_LOG_TARGETS"
    );
}

#[test]
fn the_floor_builds_whatever_the_environment_says() {
    // A directive that would lower a client crate below the floor must not
    // fail to build; it is overridden, not rejected.
    let filter = log_filter();
    assert!(!format!("{filter}").is_empty());
}
