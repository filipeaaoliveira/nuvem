use nuvem_domain::gcp::diagnostics::{
    DETAIL_MAX_LEN, GcloudDiagnostic, GcloudErrorKind, Retryability, sanitize_detail,
};

#[test]
fn sanitize_redacts_credential_shaped_content() {
    let raw = "Authorization: Bearer ya29.a0ABCDEFsecrettoken\nrefresh_token=1//longsecretvalue";
    let out = sanitize_detail(raw);
    assert!(!out.contains("ya29.a0ABCDEFsecrettoken"), "{out}");
    assert!(!out.contains("1//longsecretvalue"), "{out}");
    assert!(out.contains("[redacted]"), "{out}");
}

#[test]
fn sanitize_redacts_google_api_key() {
    let raw = "AIzaSyD-EXAMPLEfakeapikey1234567890abcd here";
    let out = sanitize_detail(raw);
    assert!(
        !out.contains("AIzaSyD-EXAMPLEfakeapikey1234567890abcd"),
        "{out}"
    );
    assert!(out.contains("[redacted]"), "{out}");
}

#[test]
fn sanitize_redacts_bare_jwt() {
    let raw = "token eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.fakeSignatureXYZ end";
    let out = sanitize_detail(raw);
    assert!(
        !out.contains(
            "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.fakeSignatureXYZ"
        ),
        "{out}"
    );
    assert!(out.contains("[redacted]"), "{out}");
}

#[test]
fn sanitize_does_not_over_redact_normal_words_and_resource_ids() {
    let raw = "describe projects/my-proj/zones/us-central1-a";
    let out = sanitize_detail(raw);
    assert_eq!(out, raw);
}

#[test]
fn sanitize_strips_control_chars_and_bounds_length() {
    let raw = format!("line one\r\n\t{}", "x".repeat(1000));
    let out = sanitize_detail(&raw);
    assert!(out.len() <= DETAIL_MAX_LEN, "len={}", out.len());
    assert!(!out.contains('\r') && !out.contains('\t'));
    assert!(!out.contains('\n'));
}

#[test]
fn sanitize_empty_is_empty() {
    assert_eq!(sanitize_detail("   "), "");
}

#[test]
fn diagnostic_headline_and_copy_text() {
    let d = GcloudDiagnostic::new(
        GcloudErrorKind::PermissionDenied,
        "GCE instances",
        Some("gcloud compute instances list".to_string()),
        Some(1),
        "PERMISSION_DENIED: caller lacks compute.instances.list",
    );
    assert_eq!(d.label(), "permission denied");
    assert_eq!(d.retryability(), Retryability::Persistent);
    assert_eq!(
        d.headline(),
        "permission denied — Check IAM roles for this account and project"
    );
    let copy = d.copy_text();
    assert!(copy.contains("GCE instances"));
    assert!(copy.contains("gcloud compute instances list"));
    assert!(copy.contains("exit code 1"));
    assert!(copy.contains("compute.instances.list"));
}

/// The command summary used to be a static literal at every call site, so it
/// reached the clipboard verbatim. A user-declared plugin's label reaches it
/// now — rendered against a row, so it can carry anything the row does — and
/// `copy_text` still claims to be fully sanitized.
#[test]
fn diagnostic_command_summary_is_sanitized_at_construction() {
    let operator_authored = "Describe web\u{7}01 with ya29.SECRETTOKEN";
    let d = GcloudDiagnostic::new(
        GcloudErrorKind::Unknown,
        "plugin",
        Some(operator_authored.to_string()),
        Some(1),
        "",
    );

    let copy = d.copy_text();
    assert!(
        !copy.contains("ya29.SECRETTOKEN"),
        "a credential-shaped value in the summary must be redacted: {copy:?}"
    );
    assert!(
        !copy.chars().any(|c| c.is_control() && c != '\n'),
        "no control byte may reach the copy text through the summary: {copy:?}"
    );
    assert!(
        copy.contains("[redacted]") && copy.contains("Describe"),
        "the summary still names the command it belongs to: {copy:?}"
    );
}

/// The bound `DETAIL_MAX_LEN` states applies to the summary too, now that the
/// summary can be operator-authored and is therefore unbounded at the source.
#[test]
fn diagnostic_command_summary_is_bounded_at_construction() {
    let d = GcloudDiagnostic::new(
        GcloudErrorKind::Unknown,
        "plugin",
        Some("a".repeat(DETAIL_MAX_LEN * 3)),
        None,
        "",
    );

    let summary = d.command_summary.expect("the summary survives sanitizing");
    assert!(
        summary.len() <= DETAIL_MAX_LEN,
        "summary was {} bytes",
        summary.len()
    );
}

#[test]
fn diagnostic_detail_is_sanitized_at_construction() {
    let d = GcloudDiagnostic::new(
        GcloudErrorKind::Unknown,
        "GCE",
        None,
        None,
        "token: Bearer ya29.SECRET",
    );
    assert!(!d.detail_preview().contains("ya29.SECRET"));
}

/// A token that MENTIONS a credential word is not a credential.
///
/// Plugin labels are operator-authored and templated against row names, so a
/// Secret Manager row called `db-password` reaches this function as ordinary
/// display text. Each case asserts byte-identical output: a rule that redacts
/// any part of these has over-redacted.
#[test]
fn sanitize_keeps_tokens_that_only_name_a_credential() {
    for kept in [
        "Describe secret-store-prod",
        "db-password",
        "the refresh_token expired",
        "https://host/secrets/db-password",
        "projects/my-proj/zones/us-central1-a",
        "password=",
    ] {
        assert_eq!(sanitize_detail(kept), kept, "for {kept:?}");
    }
}
