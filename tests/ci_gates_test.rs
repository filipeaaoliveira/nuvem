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

/// Every workflow file, as (path, contents), sorted by path. Discovered rather
/// than listed, so a workflow added later is held to the same rules without
/// anyone remembering to add it here.
fn every_workflow() -> Vec<(String, String)> {
    let mut paths: Vec<std::path::PathBuf> = std::fs::read_dir(".github/workflows")
        .expect("the workflows directory is readable")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            matches!(
                path.extension().and_then(|ext| ext.to_str()),
                Some("yml" | "yaml")
            )
        })
        .collect();
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("{} is readable: {error}", path.display()));
            (path.display().to_string(), text)
        })
        .collect()
}

/// The key of the nearest line above `index` that is indented less than it:
/// the mapping that line belongs to. Comments and blank lines are skipped.
fn parent_key(lines: &[&str], index: usize) -> Option<String> {
    let own = indent_of(lines[index]);
    lines[..index]
        .iter()
        .rev()
        .copied()
        .filter(|line| !line.trim().is_empty() && !line.trim_start().starts_with('#'))
        .find(|line| indent_of(line) < own)
        .map(|line| line.trim().trim_start_matches("- ").to_string())
}

fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// The condition that separates a real release from a rehearsal of one. Named
/// once because several places in `release.yml` carry it and a test that spelled
/// it out per assertion would let a typo in one of them pass as a guard.
const TAG_PUSH_ONLY: &str = "if: github.event_name == 'push'";

/// What the attestation step must carry instead. GitHub issues no attestation
/// for a private user-owned repository — it answers with an error, not a
/// no-op — so with the tag-push guard alone, a tag pushed to a private fork
/// fails the build at this step and takes the publish job down with it.
const TAG_PUSH_AND_PUBLIC: &str =
    "if: github.event_name == 'push' && github.event.repository.private == false";

/// The platform guards of the build job. The glibc floor is a Linux promise,
/// built and checked with Zig; the macOS legs build with plain cargo.
const LINUX_ONLY: &str = "if: runner.os == 'Linux'";
const MACOS_ONLY: &str = "if: runner.os == 'macOS'";

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
        [
            "x86_64-unknown-linux-gnu",
            "aarch64-unknown-linux-gnu",
            "aarch64-apple-darwin",
            "x86_64-apple-darwin",
        ],
        "every published platform is built in CI, natively, on a runner that can \
         also execute it: the Homebrew formula and the README name exactly these \
         four, and a target built anywhere else would ship an archive the smoke \
         test never touched and the attestation never signed: {targets:?}"
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

    // The GitHub CLI on a runner image moves with the image, and an image is
    // not obliged to carry one at all; the build job never needs it, so
    // nothing earlier would catch the gap. Pinned to a version rather than
    // tracking the image, because the tool that publishes a release must not
    // change underneath the repository without a commit.
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
        "--prerelease must hang off the hyphen case arm, so it marks v1.2.0-rc.1 \
         and leaves v1.2.0 alone: {prerelease:?}"
    );
    assert!(
        steps_guarded_by(build, TAG_PUSH_ONLY).is_empty(),
        "no step may carry the bare tag-push guard: the attestation is the only \
         step in this job that depends on the event, and it needs the \
         public-repository half too, or it fails the build on a private repository"
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

    // The glibc floor is a property of the Linux archives only, and it is
    // enforced by these steps together: Zig is installed, the binary is linked
    // against the floor, and the floor is read back off it. Each of them
    // skipped on Linux would ship a binary built against the runner's glibc;
    // any of them run on macOS fails, because Darwin has no glibc.
    assert_eq!(
        steps_guarded_by(build, LINUX_ONLY),
        BTreeSet::from([
            "Install Zig and cargo-zigbuild",
            "Show the build toolchain versions",
            "Build release binary (Linux, against the glibc floor)",
            "Check the glibc floor of the built binary",
        ]),
        "the Zig toolchain, the glibc-floor build and the floor check run on \
         the Linux legs and only there"
    );
    assert_eq!(
        steps_guarded_by(build, MACOS_ONLY),
        BTreeSet::from(["Build release binary (macOS)"]),
        "the macOS legs build with plain cargo, and nothing else is macOS-only: \
         the suite, the smoke test, packaging and the attestation run on every leg"
    );
}

/// Every job in every workflow runs on a GitHub-hosted runner, named by a
/// fixed label.
///
/// A self-hosted runner executes whatever a workflow on this repository asks
/// of it, and in a public repository that includes a pull request from a
/// fork. A runner group or a `vars.`-driven label hands that choice to
/// repository settings that no review sees. So the label is a literal, it is
/// one of GitHub's own, and a matrix that names its runners is held to the
/// same list entry by entry.
///
/// The list is GitHub's catalogue of standard hosted labels for Linux and
/// macOS. A label outside it is a larger runner or a self-hosted one, and
/// adding it here is a decision to be made on purpose.
#[test]
fn every_job_runs_on_a_github_hosted_runner() {
    const HOSTED: [&str; 13] = [
        "ubuntu-latest",
        "ubuntu-slim",
        "ubuntu-22.04",
        "ubuntu-24.04",
        "ubuntu-26.04",
        "ubuntu-22.04-arm",
        "ubuntu-24.04-arm",
        "ubuntu-26.04-arm",
        "macos-latest",
        "macos-15",
        "macos-26",
        "macos-15-intel",
        "macos-26-intel",
    ];
    // A job may take its runner from its own matrix, and the matrix entries'
    // `runs-on:` lines are then checked against the list like any other.
    const FROM_MATRIX: &str = "${{ matrix.runs-on }}";

    let workflows = every_workflow();
    assert!(
        !workflows.is_empty(),
        "no *.yml or *.yaml file found under .github/workflows; this test would \
         check nothing"
    );

    for (name, text) in &workflows {
        let lines: Vec<&str> = text.lines().collect();
        let mut from_matrix = false;
        let mut literal = 0;
        let mut runners = 0;

        for (index, line) in lines.iter().enumerate() {
            let at = format!("{name}:{}", index + 1);
            let trimmed = line.trim();
            if trimmed.starts_with('#') {
                continue;
            }

            assert!(
                !trimmed.contains("self-hosted"),
                "{at} names a self-hosted runner: {trimmed:?}"
            );

            if let Some(runner) = trimmed.strip_prefix("runs-on:").map(str::trim) {
                runners += 1;
                if runner == FROM_MATRIX {
                    from_matrix = true;
                    continue;
                }
                literal += 1;
                assert!(
                    HOSTED.contains(&runner),
                    "{at} runs a job on {runner:?}, which is not a GitHub-hosted \
                     label. Self-hosted labels, runner groups (an empty value \
                     here is a `group:`/`labels:` mapping) and `vars.` \
                     expressions are all refused: the runner must be GitHub's \
                     and must be read from the workflow itself"
                );
            }

            // A `group:` key has one legitimate home in a workflow: the
            // concurrency block. Under `runs-on:` it selects a runner group,
            // which is the indirection this test exists to refuse.
            if trimmed.starts_with("group:") || trimmed.starts_with("- group:") {
                let parent = parent_key(&lines, index);
                assert_eq!(
                    parent.as_deref(),
                    Some("concurrency:"),
                    "{at} has a `group:` key outside a `concurrency:` block \
                     (its parent is {parent:?}); a group anywhere else points \
                     a job at a runner GitHub does not host"
                );
            }
        }

        assert!(runners > 0, "{name} declares no job runner at all");
        if from_matrix {
            assert!(
                literal > 0,
                "{name} takes a runner from its matrix, but no matrix entry names one"
            );
        }
    }
}

/// One run per change: `push` is scoped to the default branch.
///
/// Unscoped, a branch with a pull request open runs this whole workflow twice
/// against the same commit -- once for the push, once for the pull request.
/// That doubles the runner time spent on every change and adds no signal: both
/// runs test the same tree.
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
