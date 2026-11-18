//! Evidence: CI gates — the shell lint, the runner label, the trigger scoping
//! and the release workflow.
//!
//! The workflow files are the one artifact in this repo that nothing else
//! checks: they are not compiled, not linted by clippy, and a mistake in them
//! does not fail until the run that was supposed to catch something. The
//! properties pinned here are the ones whose breakage is *silent* — CI stays
//! green while a gate stops gating or quietly runs twice.
//!
//! Assertions are pinned, not presence checks: each expected value is read out
//! of one part of the workflow and compared against another part, or against a
//! fact computed here. The exception is the shellcheck coverage test, which is
//! substring-shaped by necessity and says so.

use std::collections::BTreeSet;

fn workflow() -> String {
    std::fs::read_to_string(".github/workflows/ci.yml").expect("the CI workflow is readable")
}

/// The tag-driven release workflow: the one workflow that has never run, and
/// whose first run publishes something that cannot be unpublished. Nothing
/// else reads it, so its silent failure modes are pinned here.
fn release_workflow() -> String {
    std::fs::read_to_string(".github/workflows/release.yml")
        .expect("the release workflow is readable")
}

/// The condition that separates a real release from a rehearsal of one. Named
/// once because three places in `release.yml` carry it and a test that spelled
/// it out per assertion would let a typo in one of them pass as a guard.
const TAG_PUSH_ONLY: &str = "if: github.event_name == 'push'";

/// What the attestation step must carry instead. GitHub issues no attestation
/// for a user-owned private repository — it answers with an error, not a
/// no-op — so the tag-push guard alone lets the step fail the build and take
/// the publish job with it, which is how v0.1.0-rc.1 lost its release.
const TAG_PUSH_AND_PUBLIC: &str =
    "if: github.event_name == 'push' && github.event.repository.private == false";

/// The steps of a block whose first directive after `- name:` is `guard`.
/// Position, not a search of the whole block: a search would let one step's
/// condition answer for a step that carries none.
fn steps_guarded_by<'a>(block: &'a str, guard: &str) -> BTreeSet<&'a str> {
    let mut guarded = BTreeSet::new();
    let mut lines = directives(block).peekable();
    while let Some(line) = lines.next() {
        let Some(name) = line.trim().strip_prefix("- name: ") else {
            continue;
        };
        if lines.peek().is_some_and(|next| next.trim() == guard) {
            guarded.insert(name);
        }
    }
    guarded
}

/// The event keys under a workflow's `"on":` block.
fn triggers_of(source: &str) -> BTreeSet<&str> {
    directives(top_level_block(source, "\"on\""))
        .filter_map(|line| line.strip_prefix("  "))
        .filter(|rest| !rest.starts_with([' ', '-']))
        .filter_map(|rest| rest.split_once(':'))
        .map(|(key, _)| key)
        .collect()
}

/// The body of one top-level job, from its `  <name>:` header to the next
/// header at the same indentation. Job-scoped assertions need this: searching
/// the whole file would let a `continue-on-error` in some other job satisfy —
/// or falsely fail — a claim about this one.
fn job_block<'a>(workflow: &'a str, name: &str) -> &'a str {
    let header = format!("\n  {name}:\n");
    let start = workflow
        .find(&header)
        .unwrap_or_else(|| panic!("the workflow defines a `{name}` job"))
        + header.len();
    let body = &workflow[start..];
    let end = body
        .match_indices("\n  ")
        .find(|(offset, _)| {
            let rest = &body[offset + "\n  ".len()..];
            rest.starts_with(|c: char| c.is_alphanumeric())
        })
        .map_or(body.len(), |(offset, _)| offset);
    &body[..end]
}

/// The body of one top-level mapping key, from its `<name>:` header to the next
/// line that starts in column zero. `job_block` cannot serve here: triggers and
/// jobs sit at different depths, and a search of the whole file would let
/// `ci.yml`-shaped prose or a nested key answer for the top-level one.
fn top_level_block<'a>(workflow: &'a str, name: &str) -> &'a str {
    // Quoted because `on` is a YAML 1.1 boolean; both workflows write `"on":`.
    let header = format!("\n{name}:\n");
    let start = workflow
        .find(&header)
        .unwrap_or_else(|| panic!("the workflow defines a top-level `{name}` key"))
        + header.len();
    let body = &workflow[start..];
    let end = body
        .match_indices('\n')
        .find(|(offset, _)| {
            let rest = &body[offset + 1..];
            rest.starts_with(|c: char| c.is_alphanumeric() || c == '"' || c == '#')
        })
        .map_or(body.len(), |(offset, _)| offset);
    &body[..end]
}

/// The lines of a block that YAML actually acts on. Comments are excluded on
/// purpose: this file's own subject matter is `continue-on-error` and `|| true`,
/// and the workflow's comments name both while explaining why they are absent.
/// Scanning raw text would fail on the explanation rather than on the setting.
fn directives(block: &str) -> impl Iterator<Item = &str> {
    block
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'))
}

#[test]
fn shellcheck_covers_every_shell_script_in_the_repo() {
    let workflow = workflow();
    let shellcheck = job_block(&workflow, "shellcheck");

    // PRESENCE-ONLY: the CI step's arguments are shell globs expanded by the
    // runner, so the set of files it analyses is not derivable here. What is
    // derivable is that the globs are not vacuous and that every script lives
    // where one of them looks — the two ways this step could pass having
    // analysed nothing.
    let globs = linted_globs(shellcheck);
    assert!(
        !globs.is_empty(),
        "the shellcheck job must pass shellcheck something to lint"
    );

    let shell_scripts: BTreeSet<String> = repo_files()
        .into_iter()
        .filter(|path| is_shell_script(path))
        .collect();

    let matched = |path: &String| globs.iter().any(|glob| matched_by_glob(path, glob));
    assert!(
        shell_scripts.iter().any(matched),
        "the lint step's globs {globs:?} match nothing; it would analyse no code"
    );

    // EVERY glob has to match something, not just one of them. A glob pointing
    // at a directory that no longer exists expands to nothing, and shellcheck
    // exits non-zero on being handed a path it cannot read -- so the step goes
    // red for a reason that has nothing to do with the shell scripts in the
    // repository. That happened: deleting the last script under
    // `.github/scripts/` left the glob behind and reddened this gate on every
    // branch until the glob was removed.
    let vacuous: Vec<&String> = globs
        .iter()
        .filter(|glob| !shell_scripts.iter().any(|path| matched_by_glob(path, glob)))
        .collect();
    assert!(
        vacuous.is_empty(),
        "these globs match no file in the repository, so shellcheck is handed \
         a path that does not exist and the step fails on that rather than on \
         any script: {vacuous:?}"
    );
    let unmatched: Vec<&String> = shell_scripts.iter().filter(|p| !matched(p)).collect();
    assert!(
        unmatched.is_empty(),
        "these shell scripts are outside the linted globs {globs:?} and nothing \
         analyses them: {unmatched:?}"
    );
}

#[test]
fn the_fallback_walk_finds_the_shell_scripts_the_lint_gate_is_about() {
    // The test above is only as strong as the weaker of its two enumeration
    // paths, and locally it never takes the fallback. This exercises the
    // fallback directly, and against an expectation that does not route through
    // git or through `walk` itself: in a checkout with no `.git` the fallback
    // is the *only* path, so comparing it to `repo_files()` there would compare
    // a function to itself and pin nothing in the one environment that matters.
    let mut walked = Vec::new();
    walk(".".as_ref(), &mut String::new(), &mut walked);

    assert!(
        walked.iter().any(|path| path.starts_with(".github/")),
        "the fallback walk must descend into `.github/`, a dot-directory that \
         `git ls-files` reports and that CI's own shell scripts would live in"
    );

    let walked: BTreeSet<&str> = walked.iter().map(String::as_str).collect();
    let expected: BTreeSet<String> = ["scripts", ".github"]
        .into_iter()
        .flat_map(shell_scripts_under)
        .collect();

    assert!(
        !expected.is_empty(),
        "no shell scripts found under `scripts/` or `.github/`; this test would \
         assert nothing"
    );
    let missed: Vec<&String> = expected
        .iter()
        .filter(|path| !walked.contains(path.as_str()))
        .collect();
    assert!(
        missed.is_empty(),
        "the fallback walk did not enumerate these shell scripts, so the coverage \
         gate above is blind to them wherever git is unavailable: {missed:?}"
    );
}

/// Shell scripts under `dir`, read straight off disk — no git, and not through
/// `walk`. The point is an expectation the fallback can actually be measured
/// against in a tree with no git repository around it.
fn shell_scripts_under(dir: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let mut pending = vec![std::path::PathBuf::from(dir)];
    while let Some(next) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&next) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if let Some(path) = path.to_str().filter(|path| is_shell_script(path)) {
                found.insert(path.to_owned());
            }
        }
    }
    found
}

/// Every file git knows about — tracked, plus untracked ones it would add.
/// `--exclude-standard` applies `.gitignore`, so `target/` and other ignored
/// build output stay out, and git never reports `.git/` itself.
///
/// Falls back to walking the tree when there is no repository to ask — an
/// exported or copied source tree with no `.git` in it — so the coverage test
/// still runs there rather than failing on the enumeration.
fn repo_files() -> Vec<String> {
    let output = std::process::Command::new("git")
        .args([
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
        ])
        .output();
    match output {
        Ok(output) if output.status.success() => String::from_utf8(output.stdout)
            .expect("paths are UTF-8")
            .split('\0')
            .filter(|path| !path.is_empty())
            .map(str::to_owned)
            .collect(),
        _ => {
            let mut files = Vec::new();
            walk(".".as_ref(), &mut String::new(), &mut files);
            files
        }
    }
}

/// The `repo_files` fallback: every file under `dir`, as a repo-relative path.
/// Build output and VCS noise are skipped by name because there is no
/// `.gitignore` machinery here — the point is to see the repo's own sources,
/// and a copied tree's `target/` holds thousands of vendored scripts that would
/// swamp the check with findings about other people's code.
///
/// `.github/` is explicitly *not* skipped even though it is a dot-directory:
/// `git ls-files` reports it, so skipping it here would make the two
/// enumeration paths disagree about scope, and a script under `.github/` would
/// fail the coverage test locally while passing in a git-less tree.
fn walk(dir: &std::path::Path, prefix: &mut String, files: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let hidden_and_not_github = name.starts_with('.') && name != ".github";
        if hidden_and_not_github || name == "target" || name == "node_modules" {
            continue;
        }
        let restore = prefix.len();
        prefix.push_str(&name);
        if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            prefix.push('/');
            walk(&entry.path(), prefix, files);
        } else {
            files.push(prefix.clone());
        }
        prefix.truncate(restore);
    }
}

/// A shell script by extension anywhere in the repo, or — inside `scripts/`,
/// where the lint step looks — by shebang, so an extensionless helper dropped
/// next to the linted ones is not mistaken for documentation.
fn is_shell_script(path: &str) -> bool {
    if path.ends_with(".sh") || path.ends_with(".bash") {
        return true;
    }
    // Extensionless scripts are only looked for where scripts live, and
    // `.github/` is one of those places: it holds workflow-support scripts.
    if !path.starts_with("scripts/") && !path.starts_with(".github/") {
        return false;
    }
    std::fs::read_to_string(path).is_ok_and(|contents| {
        contents
            .lines()
            .next()
            .is_some_and(|line| line.starts_with("#!") && line.contains("sh"))
    })
}

/// The globs the lint step passes to shellcheck, read off the step rather than
/// restated here. A test that hardcodes `scripts/*.sh` stops being an invariant
/// the moment the step grows a second directory: it then fails for its own
/// staleness while the question it was asked — is every script analysed by
/// something — has a perfectly good answer. That is how `.github/scripts/`
/// arrived.
///
/// Anchored on `run: shellcheck `, not on the bare word: the job also names the
/// analyzer in a step title and probes for it inside a shell block, and reading
/// either as a file list is how "Ensure shellcheck is present" became a glob
/// called `is`. Flags are then dropped so the `shellcheck --version` step
/// contributes nothing; it names the analyzer, it lints no files.
fn linted_globs(shellcheck_block: &str) -> Vec<String> {
    directives(shellcheck_block)
        .filter_map(|line| line.split_once("run: shellcheck "))
        .flat_map(|(_, args)| args.split_whitespace())
        .filter(|arg| !arg.starts_with('-'))
        .map(str::to_string)
        .collect()
}

/// What the runner expands a `<dir>/*.sh` glob to: one path segment under
/// `<dir>`, ending `.sh`. A nested `scripts/lib/helper.sh` does not match.
fn matched_by_glob(path: &str, glob: &str) -> bool {
    let dir = glob.strip_suffix("/*.sh").unwrap_or_else(|| {
        panic!(
            "the lint step passes {glob:?}, a shape this test cannot expand — it \
             understands `<dir>/*.sh` only. Teach it the new shape rather than \
             leaving scripts to be checked against a glob nothing here evaluates"
        )
    });
    path.strip_prefix(dir)
        .and_then(|rest| rest.strip_prefix('/'))
        .is_some_and(|rest| !rest.contains('/') && rest.ends_with(".sh"))
}

/// Every packaged target is uploaded under a name the release job collects,
/// every file the archive step copies exists, the tag glob is the version
/// shape the manifest uses, and privilege is granted per job rather than to
/// the whole workflow.
#[test]
fn the_release_workflow_publishes_what_it_builds() {
    let release = release_workflow();

    let triggers = triggers_of(&release);
    assert_eq!(
        triggers,
        BTreeSet::from(["push", "workflow_dispatch"]),
        "the tag push publishes and the dispatch rehearses: nothing else starts \
         this workflow, and the guards below are what keep the two apart: \
         {triggers:?}"
    );
    assert!(
        release.contains("- \"v[0-9]+.[0-9]+.[0-9]+*\""),
        "the tag glob must match v<semver>, which is what the version check below compares"
    );
    assert!(
        release.contains("name: Check the tag matches Cargo.toml"),
        "a tag that disagrees with Cargo.toml must be refused before anything is built"
    );

    let targets: Vec<&str> = release
        .lines()
        .filter_map(|line| line.trim().strip_prefix("- target: "))
        .collect();
    assert_eq!(
        targets,
        ["x86_64-unknown-linux-gnu"],
        "the pool this builds on is x86_64 Linux, so the matrix lists the one \
         target it can also execute. The macOS archives are built on a Mac and \
         attached to the release by hand; a target added here without a runner \
         that can run it would ship an archive the smoke test never touched: \
         {targets:?}"
    );
    assert!(
        release.contains("name: archive-${{ matrix.target }}"),
        "every target's archive is uploaded under the archive-<target> name"
    );
    assert!(
        release.contains("pattern: archive-*"),
        "the release job collects every archive-* artifact"
    );

    let copied = release
        .lines()
        .find(|line| line.trim().starts_with("cp target/") && line.contains("/release/nuvem "))
        .expect("the package step copies the binary and its documents");
    // Split on the binary's path rather than counting tokens: that path now
    // carries a matrix expression, and `${{ matrix.target }}` is three
    // whitespace-separated words, which a token count reads as filenames.
    let documents = copied
        .split_once("/release/nuvem ")
        .map_or("", |(_, rest)| rest);
    for file in documents.split_whitespace() {
        let file = file.trim_end_matches("\"$STAGE\"/");
        if file.is_empty() || file.starts_with('$') {
            continue;
        }
        assert!(
            std::path::Path::new(file).is_file(),
            "the archive step copies {file}, which does not exist in the repository"
        );
    }

    assert!(
        release.contains("permissions: {}"),
        "workflow-level permissions are empty; each job declares its own"
    );
    let build = job_block(&release, "build");

    // A flag that is passed is not a floor that holds. The build links against
    // a named glibc and then reads the requirement back off the binary: a
    // --target suffix silently ignored, or a dependency that drags in a newer
    // symbol, would otherwise ship a binary that refuses to start on the
    // distributions the README promises, and the failure lands on whoever
    // downloads it rather than here.
    assert!(
        build.contains("--target ${{ matrix.target }}.${{ env.GLIBC_FLOOR }}"),
        "the release binary must be linked against the declared glibc floor, \
         not whatever glibc the runner image happens to carry"
    );
    // Zig and cargo-zigbuild are the compiler and linker that produce the
    // binary people download, fetched over the network at build time. Each
    // download is checked against a digest recorded in the workflow: without
    // that, whatever the network returned goes into the release, which is the
    // question the release's own .sha256 exists to answer for its users.
    for (tool, digest) in [
        ("zig.tar.xz", "ZIG_SHA256"),
        ("cargo-zigbuild.tar.xz", "CARGO_ZIGBUILD_SHA256"),
    ] {
        assert!(
            build.contains(&format!("${{{digest}}}  $tmp/{tool}\" | sha256sum -c -")),
            "the {tool} download must be verified against {digest} before it is \
             unpacked and used to build a release"
        );
    }

    assert!(
        directives(build)
            .any(|line| line.trim() == "- name: Check the glibc floor of the built binary"),
        "the build must verify the floor it promises, by reading the binary's \
         own symbols; passing the flag is not evidence that it took effect"
    );
    assert!(
        build.contains("contents: read") && build.contains("attestations: write"),
        "the build job reads the repository and signs attestations, nothing more"
    );
    let publish = job_block(&release, "release");
    assert!(
        publish.contains("contents: write"),
        "only the release job may create the release"
    );

    // The rehearsal's whole safety property: a dispatch builds and packages,
    // and cannot publish. Without this guard `gh release create` runs with a
    // branch name in GITHUB_REF_NAME and cuts a release called `main` — which
    // is exactly the unpublishable-mistake class this test exists to prevent.
    assert!(
        directives(publish).any(|line| line.trim() == TAG_PUSH_ONLY),
        "the release job must be guarded by the tag push, or a manual dispatch \
         publishes a release named after whatever branch it ran from"
    );

    // The image this job runs on is not guaranteed to carry the GitHub CLI:
    // hosted images ship one, this pool's image does not, and the first tag
    // run failed with `gh: command not found` after a green build — the build
    // job never needs it, so nothing earlier catches the gap. Pinned to a
    // version rather than tracking latest, because the tool that publishes a
    // release must not change underneath the repository without a commit.
    let publish_lines: Vec<&str> = directives(publish).collect();
    let install_at = publish_lines
        .iter()
        .position(|line| line.contains("GH_VERSION:"))
        .expect(
            "the release job must install the GitHub CLI: the runner image is not \
             guaranteed to carry one, and `gh release create` is how it publishes",
        );
    let publish_at = publish_lines
        .iter()
        .position(|line| line.contains("gh release create"))
        .expect("the release job publishes with `gh release create`");
    assert!(
        install_at < publish_at,
        "the GitHub CLI must be installed before the step that runs it"
    );
    let pinned = publish_lines[install_at]
        .split_once("GH_VERSION:")
        .map(|(_, version)| version.trim())
        .unwrap_or_default();
    assert!(
        pinned.starts_with(|c: char| c.is_ascii_digit()),
        "the GitHub CLI version must be a number this repository chose, not a \
         moving target like `latest`: {pinned:?}"
    );

    // Pinned from both sides. Missing, an rc becomes the repository's "Latest
    // release" and is handed to everyone who never reads a version number;
    // unconditional, every real release is published as a candidate.
    let prerelease = directives(publish)
        .find(|line| line.contains("--prerelease"))
        .expect("a tag carrying a pre-release identifier must be published as one");
    assert!(
        prerelease.trim_start().starts_with("*-*)"),
        "--prerelease must hang off the hyphen case arm, so it marks v0.1.0-rc.1 \
         and leaves v0.1.0 alone: {prerelease:?}"
    );
    assert!(
        steps_guarded_by(build, TAG_PUSH_ONLY).is_empty(),
        "no step may carry the bare tag-push guard: the attestation is the only \
         conditional step in this job and it needs the public-repository half \
         too, or it fails the build on a private repository"
    );
    assert_eq!(
        steps_guarded_by(build, TAG_PUSH_AND_PUBLIC),
        BTreeSet::from(["Attest build provenance"]),
        "a dispatch skips exactly one step: the attestation, which would sign \
         binaries nobody can download. The tag check is deliberately not among \
         them — it runs on both events and returns early when there is no tag, \
         so a rehearsal still proves `cargo metadata` and `jq` exist on this \
         image. Every other step is the part being rehearsed and runs on both"
    );
}

/// No workflow names a private runner group, and every job still has a runner
/// when nothing is configured.
///
/// This repository runs its jobs on a runner group of its own, and a public
/// copy must not carry that group's name: a label in a public workflow
/// advertises private infrastructure, and a fork that inherits it queues jobs
/// against runners it cannot reach, where they wait rather than fail. So the
/// label is a repository variable with a hosted fallback, and the fallback is
/// the part that matters -- an unset variable, which is what a fork, a clone
/// and a public copy all see, must still name a runner or the workflow is
/// invalid rather than merely hosted.
#[test]
fn jobs_take_their_runner_from_a_variable_with_a_hosted_fallback() {
    const HOSTED_FALLBACK: &str = "${{ vars.NUVEM_RUNNER_LABEL || 'ubuntu-latest' }}";

    let text = workflow();
    let runners: Vec<&str> = text
        .lines()
        .filter_map(|line| line.trim().strip_prefix("runs-on:"))
        .map(str::trim)
        .collect();
    assert!(!runners.is_empty(), "ci.yml declares no job runner at all");
    for runner in &runners {
        assert_eq!(
            *runner, HOSTED_FALLBACK,
            "ci.yml pins a runner directly; the label is a variable so the \
             public tree names no private group, and the fallback is what a \
             fork and a public copy run on"
        );
    }

    // The release workflow takes its runner from the same variable, and the
    // rule it must not break is this one: no private group named anywhere in a
    // tree that is going to be public.
    for (name, text) in [("ci.yml", workflow()), ("release.yml", release_workflow())] {
        for marker in ["self-hosted", "nuvem-runners"] {
            assert!(
                !text.contains(marker),
                "{name} names {marker:?}, which is private infrastructure in a \
                 tree that is going to be public"
            );
        }
    }
}

/// One run per change: `push` is scoped to the default branch.
///
/// Unscoped, a branch with a pull request open runs this whole workflow twice
/// against the same commit -- once for the push, once for the pull request.
/// On a two-runner pool that is not a duplicate, it is a queue: the second
/// copy of every job waits for the first to finish before anything else can
/// start.
///
/// The scoping is what makes `pull_request` the branch trigger and `push` the
/// default branch's own. Removing the branch filter restores the doubling
/// silently -- every check still goes green, just later -- which is why it is
/// asserted rather than left to review.
#[test]
fn ci_runs_once_per_change_rather_than_once_per_event() {
    let text = workflow();
    let trigger = text
        .split("\njobs:")
        .next()
        .expect("the workflow has a trigger block before its jobs");

    let push = trigger
        .split("  push:")
        .nth(1)
        .expect("ci.yml runs on push");
    let scope = push
        .lines()
        .take(3)
        .find(|line| line.trim_start().starts_with("branches:"))
        .unwrap_or_default();
    assert!(
        scope.contains("main"),
        "`push` must be scoped to the default branch, or every branch with a \
         PR open runs this workflow twice: {scope:?}"
    );

    assert!(
        trigger.contains("  pull_request:"),
        "`pull_request` is what covers a branch once push no longer does"
    );
}
