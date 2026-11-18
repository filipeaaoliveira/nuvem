//! Pins `classify_gcloud_stderr`'s category triggers one needle at a time.
//!
//! Each trigger substring is asserted ALONE: the classifier's arms are `||`
//! chains, and a chain is only pinned when every alternative is shown to
//! classify without help from its neighbours — an `||` that silently became
//! `&&` still passes any test whose input matches two needles at once.

use nuvem_domain::gcp::diagnostics::{
    DETAIL_MAX_LEN, DiagnosticProblem, GcloudDiagnostic, GcloudErrorKind, Retryability,
    classify_gcloud_stderr, sanitize_detail, stderr_indicates_requester_pays,
};

#[test]
fn each_unauthenticated_trigger_classifies_alone() {
    for stderr in [
        "invalid_grant",
        "please reauth to continue",
        "run gcloud auth login",
        "run auth application-default login",
        "you do not currently have an active account",
        "the supplied credentials are invalid",
        "UNAUTHENTICATED",
        "request returned (401)",
    ] {
        assert_eq!(
            classify_gcloud_stderr(stderr),
            GcloudErrorKind::Unauthenticated,
            "for {stderr:?}"
        );
    }
}

#[test]
fn each_permission_denied_trigger_classifies_alone() {
    for stderr in [
        "permission denied",
        "PERMISSION_DENIED",
        "the caller does not have permission",
        "Forbidden",
        "request returned (403)",
        "status code 403",
    ] {
        assert_eq!(
            classify_gcloud_stderr(stderr),
            GcloudErrorKind::PermissionDenied,
            "for {stderr:?}"
        );
    }
}

#[test]
fn each_api_disabled_trigger_classifies_alone() {
    for stderr in [
        "API foo.googleapis.com has not been used",
        "the API is disabled",
        "API_DISABLED",
        "SERVICE_DISABLED",
        "accessNotConfigured",
        "enable it by visiting the console",
    ] {
        assert_eq!(
            classify_gcloud_stderr(stderr),
            GcloudErrorKind::ApiDisabled,
            "for {stderr:?}"
        );
    }
}

#[test]
fn each_quota_trigger_classifies_alone() {
    for stderr in [
        "Quota exceeded",
        "RESOURCE_EXHAUSTED",
        "rate limit reached",
        "request returned (429)",
    ] {
        assert_eq!(
            classify_gcloud_stderr(stderr),
            GcloudErrorKind::Quota,
            "for {stderr:?}"
        );
    }
}

#[test]
fn each_network_trigger_classifies_alone() {
    for stderr in [
        "could not reach the server",
        "a network problem occurred",
        "the connection was reset",
        "request timed out",
        "timeout while waiting",
        "host unreachable",
        "getaddrinfo ENOTFOUND",
        "DNS resolution problem",
    ] {
        assert_eq!(
            classify_gcloud_stderr(stderr),
            GcloudErrorKind::Network,
            "for {stderr:?}"
        );
    }
}

#[test]
fn each_invalid_query_trigger_classifies_alone() {
    for stderr in [
        "INVALID_ARGUMENT",
        "invalid filter expression",
        "unparseable filter given",
    ] {
        assert_eq!(
            classify_gcloud_stderr(stderr),
            GcloudErrorKind::InvalidQuery,
            "for {stderr:?}"
        );
    }
}

#[test]
fn each_missing_project_trigger_classifies_alone() {
    for stderr in [
        "required argument [PROJECT] was not provided",
        "there is no project set",
        "the project id must be specified",
    ] {
        assert_eq!(
            classify_gcloud_stderr(stderr),
            GcloudErrorKind::MissingProject,
            "for {stderr:?}"
        );
    }
}

#[test]
fn each_invalid_input_trigger_classifies_alone() {
    for stderr in [
        "invalid argument given",
        "unrecognized arguments: --frob",
        "invalid value for flag",
        "invalid choice: 'frob'",
        "request returned (400)",
    ] {
        assert_eq!(
            classify_gcloud_stderr(stderr),
            GcloudErrorKind::InvalidInput,
            "for {stderr:?}"
        );
    }
}

#[test]
fn unmatched_stderr_stays_unknown() {
    assert_eq!(
        classify_gcloud_stderr("something else entirely"),
        GcloudErrorKind::Unknown
    );
}

// --- The two signals arrive together --------------------------------
//
// The trigger tests above feed one needle at a time, which is what makes
// them useful for pinning `||` chains and useless for pinning the *order*
// of the arms. Real gcloud stderr carries a service signal and a permission
// signal in the same message, and the arm that is tested first wins. The
// two constants below are therefore captured output, not paraphrase: a
// fixture that carries only one of the two signals cannot tell a correct
// classifier from a broken one.

/// Real `gcloud recommender recommendations list` stderr against a project
/// with the Recommender API switched off, captured 2026-08-06 during the
/// From the design pass. Byte-for-byte as gcloud printed it, except the
/// project id, renamed to `example-sandbox-1234` here and in the constant.
///
/// gcloud spells a disabled API with the gRPC code `PERMISSION_DENIED` on
/// the ERROR line and names `SERVICE_DISABLED` only in the trailing
/// ErrorInfo block. A classifier that tests "permission_denied" first sends
/// the operator to audit IAM roles that are already correct, and never
/// mentions the one thing that fixes it.
const SERVICE_DISABLED_STDERR: &str = "\
API [recommender.googleapis.com] not enabled on project
[example-sandbox-1234]. Would you like to enable and retry (this will
take a few minutes)? (y/N)?
ERROR: (gcloud.recommender.recommendations.list) PERMISSION_DENIED: Recommender \
API has not been used in project example-sandbox-1234 before or it is \
disabled. Enable it by visiting \
https://console.developers.google.com/apis/api/recommender.googleapis.com/overview \
then retry.
  reason: SERVICE_DISABLED";

/// Real stderr for a genuine IAM refusal on the same command and the same
/// day, with the project id likewise renamed to `example-gke` in comment and
/// constant alike. This is the case the fix must leave
/// alone: it is what stops "put ApiDisabled first" from swallowing every
/// permission error that mentions a service.
const USER_PROJECT_DENIED_STDERR: &str = "\
ERROR: (gcloud.recommender.recommendations.list) PERMISSION_DENIED: Caller does \
not have required permission to use project example-gke. Grant the caller the \
roles/serviceusage.serviceUsageConsumer role
  reason: USER_PROJECT_DENIED";

#[test]
fn a_disabled_api_is_not_reported_as_a_permissions_problem() {
    assert_eq!(
        classify_gcloud_stderr(SERVICE_DISABLED_STDERR),
        GcloudErrorKind::ApiDisabled,
        "gcloud spells a disabled API with the PERMISSION_DENIED gRPC code; \
         classifying on that prefix reports a switched-off API as a permissions \
         problem"
    );
}

/// The classification is a means; this is the end. An operator acts on the
/// remedy string, so the bug is only fixed when the remedy changes.
#[test]
fn a_disabled_api_tells_the_operator_to_enable_it_not_to_audit_iam() {
    assert_eq!(
        classify_gcloud_stderr(SERVICE_DISABLED_STDERR).user_action(),
        "Enable the required API in the Cloud Console"
    );
}

#[test]
fn a_real_iam_refusal_is_still_a_permissions_problem() {
    assert_eq!(
        classify_gcloud_stderr(USER_PROJECT_DENIED_STDERR),
        GcloudErrorKind::PermissionDenied,
        "a caller that genuinely lacks a role must not be told to go enable an \
         API that is already on"
    );
}

/// Each needle that outranks the permission branch must do so ON ITS OWN.
///
/// The captured fixture above carries three of them at once, which pins the
/// real-world case and pins the ordering, but cannot pin the `||` chain: with
/// any single alternative broken the other two still carry the message to
/// `ApiDisabled`. So every hoisted needle gets a message containing that
/// needle and a permission signal and nothing else, and the assertion is that
/// the service signal wins.
///
/// These inputs are constructed, not captured — they probe the predicate one
/// alternative at a time, which is a claim about this function's structure
/// rather than about gcloud's wording.
#[test]
fn each_service_needle_outranks_a_permission_signal_alone() {
    for stderr in [
        "ERROR: PERMISSION_DENIED: reason: SERVICE_DISABLED",
        "ERROR: PERMISSION_DENIED: reason: API_DISABLED",
        "ERROR: PERMISSION_DENIED: accessNotConfigured",
        "ERROR: PERMISSION_DENIED: Foo API has not been used in project 12 before",
        "ERROR: PERMISSION_DENIED: enable it by visiting the console",
    ] {
        assert_eq!(
            classify_gcloud_stderr(stderr),
            GcloudErrorKind::ApiDisabled,
            "for {stderr:?}"
        );
    }
}

/// Constructed, NOT captured — see the PR body. The real disabled-service-
/// account stderr could not be captured in this environment (every local
/// gcloud credential requires interactive reauth), so this input is a probe
/// of the ordering rule rather than evidence about gcloud's exact wording.
///
/// What it does pin is the property the rule is built on: the loose words
/// "is disabled" must not, on their own, outrank a permission signal. Every
/// needle that *does* outrank one names a service explicitly.
#[test]
fn is_disabled_alone_does_not_outrank_a_permission_signal() {
    assert_eq!(
        classify_gcloud_stderr(
            "ERROR: PERMISSION_DENIED: The service account for this project is disabled"
        ),
        GcloudErrorKind::PermissionDenied,
        "a disabled principal is not a disabled service"
    );
}

#[test]
fn each_requester_pays_trigger_detects_alone() {
    for stderr in [
        "Bucket is a requester pays bucket",
        "requester-pays access",
        "userProjectMissing",
        "user project missing on the request",
    ] {
        assert!(stderr_indicates_requester_pays(stderr), "for {stderr:?}");
    }
    assert!(!stderr_indicates_requester_pays("the bucket exists"));
}

/// Every secret-shaped token form redacts on its own, one form per token, so
/// no `||` alternative in the redaction predicate depends on a neighbour.
#[test]
fn each_secret_token_shape_redacts_alone() {
    for secret in [
        "Bearer",
        "ya29.abcdef",
        "access_token=zzz",
        "refresh_token=zzz",
        "id_token=zzz",
        "api_key=zzz",
        "apikey=zzz",
        "password=hunter2",
        "client_secret=zzz",
        "secret=zzz",
        "api_key:abc123",
        "https://host?a=1&password=hunter2",
        "{\"foo\":\"bar\",\"password\":\"hunter2\"}",
        // The credential word may only PREFIX the key; the separator does not
        // follow it immediately. Every one of these leaked while the scan
        // demanded `=`/`:` right after the word.
        "secret_key=abc123",
        "{\"secret_key\":\"abc\"}",
        // Single quotes are a separator dressing too.
        "{'password':'hunter2'}",
        // The one case pinning the scan over EVERY occurrence rather than the
        // first: the first `password` is followed by `,`, which is neither a
        // key character nor a separator, so only the second occurrence can
        // match. Its predecessor `{"password_hint":"none","password":"hunter2"}`
        // stopped discriminating once the key remainder became consumable.
        "password,x=1&password=hunter2",
        // gcloud spells its flags with hyphens, so these are the likelier
        // stderr shapes; the underscore-only word list missed all four until
        // matching moved to a hyphen-normalised copy of the token.
        "--api-key=abc123",
        "--access-token=abc123",
        "--refresh-token=abc123",
        "--id-token=abc123",
        "1//refreshvalue",
        "authorization:zzz",
        "AIzaSyExample",
        "eyJhbGciOiJ",
    ] {
        let out = sanitize_detail(&format!("before {secret} after"));
        assert_eq!(out, "before [redacted] after", "for {secret:?}");
    }
}

/// The truncation boundary is exact: a long detail is cut so the ellipsis
/// lands the byte length ON the bound, not one short of it and not over it.
#[test]
fn overlong_detail_truncates_to_exactly_the_bound() {
    let out = sanitize_detail(&"a".repeat(3 * DETAIL_MAX_LEN));
    assert_eq!(out.len(), DETAIL_MAX_LEN);
    assert!(out.ends_with('…'), "{out:?}");
    assert!(out.starts_with(&"a".repeat(DETAIL_MAX_LEN - '…'.len_utf8())));
}

#[test]
fn detail_preview_is_the_sanitized_detail() {
    let diagnostic = GcloudDiagnostic::new(
        GcloudErrorKind::Quota,
        "compute",
        None,
        Some(1),
        "quota exceeded on instances.list",
    );
    assert_eq!(
        diagnostic.detail_preview(),
        "quota exceeded on instances.list"
    );
}

/// The carrier names the binary the kind cannot know. nuvem spawns `gcloud`
/// and `bq`; this label used to read "missing gcloud CLI" whichever one was
/// absent, so a missing `bq` sent the reader to reinstall a working gcloud.
#[test]
fn a_diagnostic_that_knows_the_binary_names_it_in_label_and_action() {
    let diagnostic = GcloudDiagnostic::new(
        GcloudErrorKind::MissingExecutable,
        "BigQuery datasets",
        None,
        None,
        "No such file or directory (os error 2)",
    )
    .naming_missing_binary(Some("bq"));

    assert_eq!(diagnostic.label(), "missing bq CLI");
    assert_eq!(
        diagnostic.user_action(),
        "Install the bq CLI and ensure it is on PATH"
    );
    assert!(!diagnostic.headline().contains("gcloud"));
    assert!(diagnostic.copy_text().contains("Install the bq CLI"));
}

/// The same carrier, told gcloud, must still say gcloud — a fix that renamed
/// every missing-binary message to `bq` would pass the test above.
#[test]
fn a_diagnostic_that_knows_gcloud_was_missing_still_names_gcloud() {
    let diagnostic = GcloudDiagnostic::new(
        GcloudErrorKind::MissingExecutable,
        "gcloud",
        None,
        None,
        "No such file or directory (os error 2)",
    )
    .naming_missing_binary(Some("gcloud"));

    assert_eq!(diagnostic.label(), "missing gcloud CLI");
    assert_eq!(
        diagnostic.user_action(),
        "Install the gcloud CLI and ensure it is on PATH"
    );
    assert!(!diagnostic.headline().contains("bq"));
}

/// Naming the binary is scoped to `MissingExecutable`. Every other kind keeps
/// the kind's own words even when a binary happens to be recorded, so the
/// override cannot leak "missing bq CLI" onto a permission error.
#[test]
fn a_recorded_binary_does_not_rewrite_any_other_kind() {
    for kind in [
        GcloudErrorKind::PermissionDenied,
        GcloudErrorKind::ApiDisabled,
        GcloudErrorKind::InvalidQuery,
        GcloudErrorKind::Quota,
        GcloudErrorKind::Network,
        GcloudErrorKind::Parse,
        GcloudErrorKind::MissingProject,
        GcloudErrorKind::Unknown,
        GcloudErrorKind::Unauthenticated,
        GcloudErrorKind::InvalidInput,
        GcloudErrorKind::Cancelled,
    ] {
        let diagnostic = GcloudDiagnostic::new(kind, "service", None, None, "detail")
            .naming_missing_binary(Some("bq"));
        assert_eq!(diagnostic.label(), kind.label(), "{kind:?}");
        assert_eq!(diagnostic.user_action(), kind.user_action(), "{kind:?}");
    }
}

/// A carrier that was never told which binary was missing stays vague rather
/// than guessing. Vague is the fail-safe direction; naming gcloud was the
/// wrong-answer direction.
#[test]
fn a_carrier_with_no_binary_names_none() {
    let diagnostic = GcloudDiagnostic::new(
        GcloudErrorKind::MissingExecutable,
        "service",
        None,
        None,
        "not found",
    );

    assert_eq!(diagnostic.label(), "missing CLI");
    assert!(!diagnostic.headline().contains("gcloud"));
    assert!(!diagnostic.headline().contains("bq"));
}

/// `DiagnosticProblem` is the gcloud probe's carrier and renders the same
/// claim, including across the `GcloudDiagnostic` -> `DiagnosticProblem`
/// conversion the storage and resource state use.
#[test]
fn a_problem_carries_and_renders_the_missing_binary() {
    let problem = DiagnosticProblem::missing_executable("bq", "No such file or directory");

    assert_eq!(problem.kind, GcloudErrorKind::MissingExecutable);
    assert_eq!(problem.label(), "missing bq CLI");
    assert_eq!(
        problem.user_action(),
        "Install the bq CLI and ensure it is on PATH"
    );

    let converted = DiagnosticProblem::from(
        &GcloudDiagnostic::new(
            GcloudErrorKind::MissingExecutable,
            "BigQuery datasets",
            None,
            None,
            "No such file or directory",
        )
        .naming_missing_binary(Some("bq")),
    );
    assert_eq!(converted.missing_binary, Some("bq"));
    assert_eq!(converted.label(), "missing bq CLI");
}

/// Two failures that differ only in which binary was missing are the same
/// *kind*: that is what `GcloudErrorKind` is for, and it is the property a
/// payload on the enum would have destroyed.
#[test]
fn the_binary_is_not_part_of_the_classification() {
    let bq = GcloudDiagnostic::new(GcloudErrorKind::MissingExecutable, "s", None, None, "d")
        .naming_missing_binary(Some("bq"));
    let gcloud = GcloudDiagnostic::new(GcloudErrorKind::MissingExecutable, "s", None, None, "d")
        .naming_missing_binary(Some("gcloud"));

    assert_eq!(bq.kind, gcloud.kind);
    assert_eq!(bq.retryability(), gcloud.retryability());
    assert_ne!(bq.label(), gcloud.label());
}

/// What the real `bq` wordings classify as. A measurement, not a wish.
///
/// Every needle in this classifier was derived from gcloud's phrasing. `bq`
/// prefixes its own — "BigQuery error in <op> operation: Not found: ..." — and
/// none of the arms was written for it, so all three of the wordings captured
/// from the live binary on 2026-08-25 fall through to `Unknown`, which is
/// `Transient`: the poll ticker will keep re-asking for a table that has been
/// deleted.
///
/// That is worth stating plainly, because the later fix does *not* fix it. The
/// argument was that the classifier was being handed an empty string
/// instead of "the classification the real message would have produced" — and
/// for these three wordings the two are the same answer. What the fix buys is
/// the operator seeing the reason; the kind was `Unknown` before and is
/// `Unknown` after. Closing the gap means a not-found category, which is a new
/// `GcloudErrorKind` variant, a retryability decision for every gcloud message
/// carrying "not found" as well, and a UI label — a change with its own blast
/// radius, and not one to bury in a stdout fix.
///
/// So this test is a tripwire rather than a wish: it holds until someone
/// teaches an arm to match these, and the red it produces is the prompt to
/// decide the retryability deliberately instead of inheriting it.
///
/// There is deliberately no 403 case here. `bq`'s permission wording has never
/// been captured — the account that ran the probes could reach everything it
/// asked about — and a case invented from the shape of the others would read
/// as observed evidence to whoever came next.
#[test]
fn the_captured_bq_wordings_are_not_classified_by_any_arm() {
    for wording in [
        "BigQuery error in show operation: Not found: Table p:austin_bikeshare.no_such_table",
        "BigQuery error in ls operation: Not found: Dataset p:no_such_dataset",
        "BigQuery error in ls operation: Project 'projects/p' not found or deleted.",
    ] {
        assert_eq!(
            classify_gcloud_stderr(wording),
            GcloudErrorKind::Unknown,
            "an arm now matches this bq wording — decide its retryability on \
             purpose: {wording}"
        );
    }

    // The half that makes the sentence above a claim rather than a shrug: the
    // empty string the failure path used to hand this function classifies the
    // same way, so restoring the message changed what the operator reads and
    // nothing about what the poll ticker does.
    assert_eq!(classify_gcloud_stderr(""), GcloudErrorKind::Unknown);
    assert_eq!(
        GcloudErrorKind::Unknown.retryability(),
        Retryability::Transient
    );
}
