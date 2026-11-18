//! Evidence — the gate script requires process isolation rather than
//! preferring it.
//!
//! The active theme is a process-global arc-swap (`src/config/theme.rs`), and
//! `tests/suite.rs` mounts every integration test into one binary. Under
//! libtest that binary runs its tests as threads of one process, so a test
//! that stores a theme and a test that renders from one are not isolated:
//! `config_evidence::render_config_driven_frames_to_html` failed exactly once
//! that way once and has passed every run since. nextest gives each
//! test its own process and removes the hazard entirely.
//!
//! The gate script used to *prefer* nextest and fall back to libtest with a
//! printed note. That is the shape this file exists to prevent, because of
//! what the fallback does rather than what it says: it runs the suite in the
//! one configuration the note warns about, and then reports the tier green.
//! A gate whose degraded mode is indistinguishable from its passing mode
//! reports coverage it did not have.
//!
//! Two properties are pinned:
//!
//! 1. The gate script does not run the suite under libtest. `cargo test --doc`
//!    is the exception and the only one: nextest runs no doctests, so that run
//!    is additional coverage rather than a fallback, and it starts no test that
//!    touches the theme.
//! 2. The script refuses outright when nextest is absent, rather than
//!    proceeding degraded.
//!
//! What this file does NOT check, because reading text cannot: that the
//! script behaves as written when executed. It is a tripwire against the
//! fallback returning — through someone re-softening the refusal — not a
//! proof that the gates run.

/// The gate scripts. Named here rather than discovered on purpose: if the gate
/// is ever repointed at a different script, this test should fail and be
/// updated deliberately, not follow a manifest to a script nobody checked.
const TIER_SCRIPTS: [&str; 1] = ["scripts/check-fast.sh"];

fn script(path: &str) -> String {
    std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("the gate script {path} is readable: {error}"))
}

/// The script's command lines: whole-line comments dropped and backslash
/// continuations joined, so a multi-line invocation is one string to search.
///
/// Whole-line comments only. Stripping from the first `#` on a code line would
/// need to know which `#` are inside quotes, and these scripts carry their
/// arguments in quotes; a comment line is the case that actually matters here,
/// because the script discusses `cargo test` in prose and a prose mention must
/// not satisfy — or falsely fail — a claim about what runs.
fn commands(source: &str) -> Vec<String> {
    let joined = source.replace("\\\n", " ");
    joined
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_string)
        .collect()
}

/// True when `line` invokes `cargo test` — as opposed to `cargo nextest`,
/// which shares the prefix `cargo `. Anchored on the whole word so
/// `cargo nextest run` is never read as a libtest run.
fn is_cargo_test(line: &str) -> bool {
    line.split_whitespace()
        .collect::<Vec<_>>()
        .windows(2)
        .any(|pair| pair == ["cargo", "test"])
}

#[test]
fn no_tier_script_runs_the_suite_under_libtest() {
    for path in TIER_SCRIPTS {
        let source = script(path);
        for line in commands(&source).iter().filter(|line| is_cargo_test(line)) {
            assert!(
                line.contains("--doc"),
                "{path} runs `{line}`, which is the consolidated suite under \
                 libtest: one process, tests as threads, and the process-global \
                 active theme raced across them. Only `cargo test --doc` \
                 belongs here — nextest runs no doctests, so that run adds \
                 coverage instead of removing isolation."
            );
        }
    }
}

#[test]
fn each_tier_script_refuses_when_nextest_is_absent() {
    for path in TIER_SCRIPTS {
        let source = script(path);
        let lines = commands(&source);
        let guard = lines
            .iter()
            .position(|line| line.contains("! cargo nextest --version"))
            .unwrap_or_else(|| {
                panic!(
                    "{path} tests for cargo-nextest with `if ! cargo nextest \
                     --version`, so that its absence stops the tier. Found no \
                     such guard: without one the tier either assumes nextest \
                     or falls back to the shared-process run it exists to avoid"
                )
            });
        let closed = lines[guard..]
            .iter()
            .position(|line| line == "fi")
            .expect("the nextest guard is closed");
        assert!(
            lines[guard..guard + closed]
                .iter()
                .any(|line| line == "exit 1"),
            "{path} guards on cargo-nextest but its block does not `exit 1`. \
             A guard that reports and continues is the fallback under another \
             name: the suite still runs in one process and the tier still \
             closes green."
        );
    }
}
