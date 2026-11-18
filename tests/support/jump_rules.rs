//! Shared fixture support for the `[[jumps]]` test binaries.
//!
//! `tests/resource_jumps_test.rs`, `tests/resource_jump_picker_test.rs` and
//! `tests/jump_rules_evidence.rs` each install rule sets the same way, and they
//! all pull this file in with `#[path = "support/jump_rules.rs"]`, so it lives
//! in a subdirectory: a plain `tests/*.rs` file would become a test target of
//! its own.
//!
//! Each binary uses a different subset of what is here, so the unused half
//! would otherwise be reported as dead code in whichever one does not call it.
#![allow(dead_code)]

use nuvem::app::AppState;
use nuvem::config::jumps::JumpRules;

/// Installs a `[[jumps]]` body as the app's rule set, parsed exactly as the
/// config file's is.
///
/// Going through the real parser rather than hand-building `JumpRule` values is
/// the point: a test that constructed rules directly would keep passing if the
/// grammar a user actually writes produced something else.
pub(crate) fn with_rules(app: &mut AppState, body: &str) {
    #[derive(serde::Deserialize)]
    struct Doc {
        jumps: toml::Value,
    }
    let doc: Doc = toml::from_str(body).expect("test fixture must be valid TOML");
    let mut warnings = Vec::new();
    app.root.jump_rules = JumpRules::from_file(&doc.jumps, &mut warnings);
    assert!(
        warnings.is_empty(),
        "fixture rules must be valid: {warnings:?}"
    );
}
