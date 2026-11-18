//! Copy-diagnostic key (issue #31 AC #6): `Y` copies the last recorded
//! gcloud diagnostic's sanitized, bounded technical detail to the
//! clipboard, or reports there is nothing to copy.

use nuvem::app::reducer::root::reduce;
use nuvem::app::state::root::AppState;
use nuvem::app::{Action, Effect, EffectResult, SystemAction};
use nuvem_domain::gcp::diagnostics::{
    DiagnosticProblem, GcloudDiagnostic, GcloudDiagnosticsReport, GcloudErrorKind,
};

#[test]
fn copy_diagnostic_with_no_diagnostic_sets_status_and_emits_no_effect() {
    let mut app = AppState::new();
    let effects = reduce(&mut app, &Action::System(SystemAction::CopyDiagnostic));
    assert!(effects.is_empty());
    assert_eq!(
        app.root.system.status_message.as_deref(),
        Some("No diagnostic to copy")
    );
}

#[test]
fn copy_diagnostic_with_a_last_diagnostic_emits_sanitized_copy_text() {
    let mut app = AppState::new();
    let diagnostic = GcloudDiagnostic::new(
        GcloudErrorKind::PermissionDenied,
        "GCE instances",
        Some("gcloud compute instances list".to_string()),
        Some(1),
        "permission denied: Bearer ya29.SECRET-token",
    );
    app.root.system.set_last_diagnostic(diagnostic.clone());

    let effects = reduce(&mut app, &Action::System(SystemAction::CopyDiagnostic));

    assert_eq!(effects.len(), 1);
    match &effects[0] {
        Effect::CopyToClipboard { value, description } => {
            assert_eq!(value, &diagnostic.copy_text());
            assert_eq!(description, "diagnostic");
            // AC #3 anti-leak: the copied text is the sanitized carrier's
            // own copy_text(), which never contains the raw secret.
            assert!(!value.contains("ya29.SECRET-token"));
        }
        other => panic!("expected Effect::CopyToClipboard, got {other:?}"),
    }
}

/// FIX 4 (AC#6): the health probe's problem must also become copyable via
/// `Y`, not just resource-load failures. When `GcloudDiagnosticsProbed`
/// carries a problem, the reducer must record it as `last_diagnostic`.
#[test]
fn diagnostics_probe_with_a_problem_sets_last_diagnostic_and_is_copyable() {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::GcloudDiagnosticsProbed {
            report: GcloudDiagnosticsReport {
                available: true,
                active_account: None,
                problem: Some(DiagnosticProblem::new(
                    GcloudErrorKind::Unauthenticated,
                    "permission denied: Bearer ya29.SECRET-token",
                )),
            },
        }),
    );

    let diag = app
        .root
        .system
        .last_diagnostic()
        .expect("last_diagnostic should be set after a probe with a problem");
    assert_eq!(diag.kind, GcloudErrorKind::Unauthenticated);
    // AC #3 anti-leak: sanitize_detail is the sole chokepoint; the sensitive
    // token must never appear in the recorded diagnostic's detail.
    assert!(!diag.detail.contains("ya29.SECRET-token"));
    assert!(diag.detail.contains("[redacted]"));

    let effects = reduce(&mut app, &Action::System(SystemAction::CopyDiagnostic));
    assert_eq!(effects.len(), 1);
    match &effects[0] {
        Effect::CopyToClipboard { value, .. } => {
            assert!(!value.contains("ya29.SECRET-token"));
        }
        other => panic!("expected Effect::CopyToClipboard, got {other:?}"),
    }
}

/// A probe with no problem must leave `last_diagnostic` untouched.
#[test]
fn diagnostics_probe_without_a_problem_leaves_last_diagnostic_untouched() {
    let mut app = AppState::new();
    let existing = GcloudDiagnostic::new(
        GcloudErrorKind::PermissionDenied,
        "GCE instances",
        None,
        None,
        "permission denied",
    );
    app.root.system.set_last_diagnostic(existing.clone());

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::GcloudDiagnosticsProbed {
            report: GcloudDiagnosticsReport {
                available: true,
                active_account: Some("dev@example.com".to_string()),
                problem: None,
            },
        }),
    );

    assert_eq!(app.root.system.last_diagnostic(), Some(&existing));
}

/// The probe's problem carries the missing binary, and the reducer must carry
/// it onto `last_diagnostic` — that is the copy (`Y`) and the status message
/// the operator reads. Without it the recorded diagnostic silently drops back
/// to "missing CLI" while the report it was built from knew the name.
#[test]
fn a_probed_missing_binary_reaches_last_diagnostic_by_name() {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::GcloudDiagnosticsProbed {
            report: GcloudDiagnosticsReport {
                available: false,
                active_account: None,
                problem: Some(DiagnosticProblem::missing_executable(
                    "bq",
                    "No such file or directory (os error 2)",
                )),
            },
        }),
    );

    let diag = app
        .root
        .system
        .last_diagnostic()
        .expect("last_diagnostic should be set after a probe with a problem");
    assert_eq!(diag.kind, GcloudErrorKind::MissingExecutable);
    assert_eq!(diag.label(), "missing bq CLI");
    assert_eq!(
        diag.user_action(),
        "Install the bq CLI and ensure it is on PATH"
    );
    assert!(!diag.copy_text().contains("gcloud CLI"));
}
