//! The no-payload guarantee for Secret Manager.
//!
//! nuvem browses Secret Manager metadata. A secret's payload can never render,
//! and the reason is not that a render layer redacts it: it is that the call
//! which would fetch one **does not exist anywhere in this binary**. That is the
//! posture copied from tgcp, and it is the point of the whole service.
//!
//! nuvem already redacts secrets at the render layer — `redact_secrets` for
//! Cloud SQL, `sanitize` for diagnostics. Those exist because those services
//! legitimately fetch data that may carry credential-shaped content, so the
//! defence has to be downstream. Secret Manager is the case where it does not
//! have to be, so it isn't: this extends the guarantee from "redacted on the way
//! out" to "never fetched in the first place".
//!
//! A guarantee stated only in a comment is not a guarantee. These tests are what
//! make it one, from two directions that fail differently:
//!
//! 1. [`no_secret_argv_can_reach_the_payload_subcommand`] enumerates every argv
//!    the secrets service can construct and asserts none reaches the payload
//!    subcommand. This is the stronger of the two: it survives a rename, because
//!    the argv is what crosses the process boundary whatever the Rust symbols
//!    are called.
//! 2. [`the_payload_call_appears_nowhere_in_the_source_tree`] scans the whole of
//!    `src/` and `crates/` for the forbidden tokens. This is the broader one: it
//!    catches the call being added somewhere the argv test never looks.
//!
//! Neither subsumes the other. The scan is the one a rename walks past; the argv
//! test is the one that only sees the service module's own commands.
//!
//! # What is still uncovered
//!
//! The two layers above share a blind spot, and it is worth naming rather than
//! leaving a reader to believe the pair is airtight. The argv test only drives
//! the methods [`every_secret_argv`] lists by hand, and the whole-tree scan
//! matches the payload verb only in the joined shape `versions <verb>` — the
//! verb alone is an ordinary English word that occurs in unrelated identifiers
//! like `uniform_bucket_level_access`, so scanning for it bare across the tree
//! would fire constantly. A method that builds the verb as its own argv element
//! (`["secrets", "versions", "<verb>", name]`) therefore spells no joined
//! literal, and if it is not added to [`every_secret_argv`] the argv test never
//! drives it either.
//!
//! [`the_payload_verb_is_no_string_literal_in_the_secrets_service`] closes that
//! route where it matters: within the secrets service module, where such a
//! method would live, the verb may appear in no string literal at all —
//! separately or joined. What remains uncovered is the same construction placed
//! in some *other* module and not enumerated here. Nothing in this file catches
//! that; the module boundary is the seam, and a reviewer is what stands in it.
//!
//! # Why this file does not contain the tokens it forbids
//!
//! A whole-tree scan that ran over this file would match its own source and fail
//! forever. The two usual fixes are to exclude a path or to assemble the token
//! at runtime, and this file does the **second**: [`payload_method_token`] and
//! [`payload_subcommand`] build the strings from fragments, so the literals
//! never appear here.
//!
//! That is deliberate and it is the safer of the two. Excluding `tests/` — or
//! any directory — to make the scan pass would create a blind spot inside
//! exactly the tree a future author might add the call to. There is no exclusion
//! list anywhere in this file, and there should never be one.
//!
//! The same reasoning is why `src/` and `crates/` spell the tokens nowhere
//! either, not even in prose: the scan does no comment parsing, so a comment
//! that named the call would have to be special-cased, and every special case is
//! a crack. See the module comment in `src/services/resources/secrets.rs`.

use nuvem::services::gcloud::cli::{CommandError, CommandOutput, CommandRunner, GcloudCommand};
use nuvem::services::resources::secrets::GcloudSecretsService;
use nuvem_domain::gcp::configuration::{GcloudConfigurationName, GcloudContext};
use std::cell::RefCell;
use std::path::{Path, PathBuf};

/// The Secret Manager API method that returns a payload, assembled so the
/// literal does not appear in this file. See the module comment.
fn payload_method_token() -> String {
    format!("{}{}{}", "Access", "Secret", "Version")
}

/// The same method as the native client spells it, assembled for the same
/// reason: the generated client exposes it as a snake_case builder, and the
/// scan has to refuse that spelling too now that reads have a second backend.
fn client_payload_method_token() -> String {
    format!("{}{}{}", "access_", "secret_", "version")
}

/// The gcloud subcommand that returns a payload, assembled for the same reason.
/// Returned as the bare verb: it is one argv element, and the argv test looks
/// for it element-wise rather than in a joined string.
fn payload_subcommand() -> String {
    format!("{}{}", "acc", "ess")
}

fn ctx() -> GcloudContext {
    GcloudContext {
        configuration: GcloudConfigurationName::new("dev"),
        account: None,
        project: Some("dev-project".into()),
        region: None,
        zone: None,
    }
}

/// Records every argv it is handed and answers with an empty JSON list, so a
/// caller can drive the whole service and then inspect what it tried to run.
struct ArgvRecorder {
    seen: RefCell<Vec<Vec<String>>>,
}

impl ArgvRecorder {
    fn new() -> Self {
        Self {
            seen: RefCell::new(Vec::new()),
        }
    }
}

impl CommandRunner for ArgvRecorder {
    fn run(&self, command: GcloudCommand) -> Result<CommandOutput, CommandError> {
        self.seen.borrow_mut().push(command.args.clone());
        Ok(CommandOutput {
            // Parses as an empty list and as an all-defaults object, so one
            // canned answer serves every entry point here.
            stdout: "[]".to_string(),
            stderr: String::new(),
        })
    }
}

/// Every entry point on the secrets service, driven so its argv is recorded.
///
/// If a method is added to `GcloudSecretsService` and not called here, this test
/// stops covering it. That is the one seam in this file that a human has to
/// maintain. Within this module it is covered anyway, from the other side:
/// [`the_payload_verb_is_no_string_literal_in_the_secrets_service`] forbids the
/// verb as a string literal in the secrets service source, so an unenumerated
/// method cannot smuggle it in as a separate argv element. See the module
/// comment for what that still leaves open.
fn every_secret_argv() -> Vec<Vec<String>> {
    let context = ctx();

    let list = GcloudSecretsService::new(ArgvRecorder::new());
    let _ = list.load_secrets(&context, None);
    let _ = list.load_secrets(&context, Some("-lenv=prod"));

    let describe = GcloudSecretsService::new(ArgvRecorder::new());
    let _ = describe.load_secret_detail(&context, "api-signing-key");

    let versions = GcloudSecretsService::new(ArgvRecorder::new());
    let _ = versions.load_secret_versions(&context, "api-signing-key");

    [list, describe, versions]
        .iter()
        .flat_map(|service| service.runner().seen.borrow().clone())
        .collect()
}

#[test]
fn no_secret_argv_can_reach_the_payload_subcommand() {
    let forbidden = payload_subcommand();
    let argvs = every_secret_argv();

    assert!(
        !argvs.is_empty(),
        "the service produced no commands at all, so this test proves nothing"
    );

    for argv in &argvs {
        assert!(
            !argv.iter().any(|arg| arg == &forbidden),
            "a secrets command reaches the payload subcommand: {argv:?}"
        );
    }
}

/// The positive half of the argv contract: the service issues exactly the three
/// metadata subcommands and nothing else.
///
/// Without this, [`no_secret_argv_can_reach_the_payload_subcommand`] could be
/// satisfied by a service that had stopped issuing commands at all, and a
/// fourth, unreviewed subcommand could appear without anything noticing.
#[test]
fn the_secrets_service_issues_only_the_three_metadata_subcommands() {
    let verbs: Vec<Vec<String>> = every_secret_argv()
        .into_iter()
        .map(|argv| {
            argv.into_iter()
                .skip_while(|arg| arg != "secrets")
                .take_while(|arg| !arg.starts_with("--"))
                .collect()
        })
        .collect();

    let expected: Vec<Vec<String>> = [
        // Twice: unfiltered, then with a label filter pushed down. The pushed
        // expression rides in a `--filter=` element, so it adds no verb here —
        // which is the point, since a verb is what would reach a payload.
        vec!["secrets", "list"],
        vec!["secrets", "list"],
        vec!["secrets", "describe", "api-signing-key"],
        vec!["secrets", "versions", "list", "api-signing-key"],
    ]
    .into_iter()
    .map(|argv| argv.into_iter().map(str::to_string).collect())
    .collect();

    assert_eq!(
        verbs, expected,
        "the secrets service must issue exactly the metadata subcommands"
    );
}

/// The payload verb may appear in no string literal in the secrets service.
///
/// This is the narrow scan the whole-tree one cannot be: bare, the verb is an
/// ordinary word (`uniform_bucket_level_access`), so the tree-wide scan can only
/// afford to look for it joined to `versions`. Inside this one module there is
/// no legitimate use of it at all, joined or not, which makes the bare form
/// affordable here — and the bare form is what catches an argv built element by
/// element, the shape both other layers walk past.
#[test]
fn the_payload_verb_is_no_string_literal_in_the_secrets_service() {
    let path = Path::new("src/services/resources/secrets.rs");
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("{} is readable: {error}", path.display()));

    // Guard against the scan going vacuous if the module is moved or renamed:
    // an empty file, or the wrong one, would pass silently.
    assert!(
        text.contains("GcloudSecretsService"),
        "{} is not the secrets service module — this scan is looking at the \
         wrong file and would pass vacuously",
        path.display()
    );

    let quoted = format!("\"{}\"", payload_subcommand());
    let offenders: Vec<&str> = text
        .lines()
        .filter(|line| line.contains(&quoted))
        .map(str::trim)
        .collect();

    assert!(
        offenders.is_empty(),
        "the payload verb appears as a string literal in {}; the argv test only \
         drives the methods it enumerates, so a literal here is how the payload \
         call gets built without either other layer noticing:\n{}",
        path.display(),
        offenders.join("\n")
    );
}

/// Every `.rs` file under `root`, recursively.
fn rust_sources(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let entries = std::fs::read_dir(&directory)
            .unwrap_or_else(|error| panic!("{} is readable: {error}", directory.display()));
        for entry in entries {
            let path = entry.expect("a readable directory entry").path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                found.push(path);
            }
        }
    }
    found
}

#[test]
fn the_payload_call_appears_nowhere_in_the_source_tree() {
    // Whole tree, not just the secrets module. The guarantee this ticket makes
    // is about the binary: a scan narrowed to one module would pass happily the
    // day someone reaches for the payload call from a helper somewhere else.
    //
    // There is deliberately no exclusion list. See the module comment for why
    // this file does not need one.
    let roots = [
        Path::new("src"),
        Path::new("../crates"),
        Path::new("crates"),
    ];
    let sources: Vec<PathBuf> = roots
        .iter()
        .filter(|root| root.is_dir())
        .flat_map(|root| rust_sources(root))
        .collect();

    assert!(
        sources.len() > 50,
        "only {} source files found — the scan is looking in the wrong place \
         and would pass vacuously",
        sources.len()
    );

    let method = payload_method_token();
    let client_method = client_payload_method_token();
    let subcommand = payload_subcommand();
    // The subcommand verb alone is an ordinary English word that appears in
    // unrelated identifiers (`uniform_bucket_level_access`,
    // `public_access_prevention`), so the scan looks for it in the one shape
    // that means the payload call: the secret-version command that carries it.
    let versions_then_access = format!("versions {subcommand}");

    let mut offenders = Vec::new();
    for path in &sources {
        let text = std::fs::read_to_string(path)
            .unwrap_or_else(|error| panic!("{} is readable: {error}", path.display()));
        for token in [&method, &client_method, &versions_then_access] {
            if text.contains(token.as_str()) {
                offenders.push(format!("{}: {token}", path.display()));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "the payload-fetching call must appear nowhere in the shipped source; \
         nuvem browses Secret Manager metadata only, and this guarantee is \
         structural rather than a redaction applied later:\n{}",
        offenders.join("\n")
    );
}

/// The native secrets service, on its own: the tree-wide scan above covers it,
/// but this names the file so a reader of SECURITY.md finds the pin beside the
/// module it protects. The vacuity guard fails if the file stops being the
/// service, so a rename cannot leave this passing over nothing.
#[test]
fn the_native_secrets_service_never_spells_the_payload_call() {
    let text = std::fs::read_to_string("src/services/sdk/secrets.rs")
        .expect("the native secrets service is readable");
    assert!(
        text.contains("SdkSecretsService"),
        "src/services/sdk/secrets.rs no longer holds the native secrets service; \
         move this pin with it"
    );
    for token in [client_payload_method_token(), payload_method_token()] {
        assert!(
            !text.contains(&token),
            "the native secrets service spells the payload call {token:?}; nuvem \
             reads Secret Manager metadata only"
        );
    }
}
