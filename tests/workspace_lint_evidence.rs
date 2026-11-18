//! Evidence — every gating clippy run lints the whole workspace.
//!
//! `cargo clippy` without `--workspace` lints the root package only. This repo
//! has a second package, `crates/nuvem-domain`, and for as long as the
//! gating invocations omitted the flag, a lint failure inside that crate was
//! invisible to every gate while `cargo clippy --workspace` — what a developer
//! actually types — failed. Two deliberately-invalid regex literals sat in that
//! state from an early commit until the lint was fixed, and were rediscovered
//! as a fresh alarm four separate times.
//!
//! The recurrence is the cost being pinned here, not the two literals. A gate
//! narrower than the thing it gates does not report the gap; it reports green.
//!
//! What this cannot check, because reading text cannot: that the scripts behave
//! as written when executed. It is a tripwire against the flag being dropped
//! again, in any one of these places, while the others keep the claim
//! looking true.

/// Every file that publishes a `cargo clippy` invocation — the two gates, and
/// the two documents that tell a developer what to type. The documents are here
/// because narrowing one reproduces the defect exactly: someone runs the published
/// command, it passes, and the second package went unlinted.
///
/// Listed rather than discovered: a site added somewhere this test does not look
/// is a change that should be made deliberately, and adding it here is how.
const LINT_SITES: [&str; 4] = [
    "scripts/check-fast.sh",
    ".github/workflows/ci.yml",
    "CONTRIBUTING.md",
    "ARCHITECTURE.md",
];

fn source(path: &str) -> String {
    std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("the lint site {path} is readable: {error}"))
}

/// The `cargo clippy` invocations in one file: backslash continuations joined,
/// so a wrapped invocation is one string.
///
/// A line counts only when `cargo clippy` starts it, or follows a YAML `run:`
/// key. Both scripts, the workflow and the two documents *discuss* clippy in
/// prose and in step labels — `step "cargo clippy (deny warnings)"` is a
/// heading, not a run — and a mention must neither satisfy nor falsely fail a
/// claim about what is actually invoked.
fn clippy_invocations(source: &str) -> Vec<String> {
    let joined = source.replace("\\\n", " ");
    joined
        .lines()
        .map(str::trim)
        .filter_map(|line| {
            let command = line.strip_prefix("- ").unwrap_or(line);
            let command = command.strip_prefix("run:").unwrap_or(command).trim_start();
            command
                .starts_with("cargo clippy")
                .then(|| command.to_string())
        })
        .collect()
}

#[test]
fn every_gating_clippy_run_lints_the_workspace() {
    for path in LINT_SITES {
        let invocations = clippy_invocations(&source(path));
        assert!(
            !invocations.is_empty(),
            "{path} carries no `cargo clippy` invocation — either the gate was \
             removed or it moved somewhere LINT_SITES does not look"
        );
        for invocation in invocations {
            assert!(
                invocation.contains("--workspace"),
                "{path} lints with `{invocation}`, which covers the root \
                 package only. A clippy failure in crates/nuvem-domain would \
                 go unreported here."
            );
        }
    }
}

/// The second half of the same property: the crate that the missing
/// `--workspace` used to hide has to still be a workspace member, or the flag
/// covers nothing and the test above passes on a technicality.
#[test]
fn the_second_package_is_a_workspace_member() {
    let manifest = source("Cargo.toml");
    assert!(
        manifest.contains("crates/nuvem-domain"),
        "Cargo.toml no longer lists crates/nuvem-domain as a workspace member, \
         so `--workspace` no longer reaches the code this was about"
    );
}
