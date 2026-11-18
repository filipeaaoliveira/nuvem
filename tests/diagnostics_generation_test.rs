use nuvem::app::state::diagnostics::{DiagnosticsPhase, OverviewDiagnostics};
use nuvem_domain::gcp::diagnostics::GcloudDiagnosticsReport;

/// Reproduces the stuck-InFlight-guard scenario: a probe result that is
/// dropped because it belongs to a superseded context-generation must still
/// release the `is_in_flight()` guard, otherwise no future probe can ever
/// run again for the session.
#[test]
fn abandon_releases_guard_without_prior_report() {
    let mut diagnostics = OverviewDiagnostics::default();
    diagnostics.begin();
    assert!(diagnostics.is_in_flight());

    diagnostics.abandon();

    assert_eq!(diagnostics.phase(), DiagnosticsPhase::NotRun);
    assert!(!diagnostics.is_in_flight());
    assert!(diagnostics.report().is_none());
}

#[test]
fn abandon_releases_guard_and_preserves_prior_report() {
    let mut diagnostics = OverviewDiagnostics::default();
    diagnostics.begin();
    diagnostics.complete(GcloudDiagnosticsReport {
        available: true,
        active_account: Some("dev@example.com".to_string()),
        problem: None,
    });
    assert_eq!(diagnostics.phase(), DiagnosticsPhase::Complete);

    // A new probe begins (e.g. the user re-enters Home), then the
    // configuration changes before the stale probe's result arrives.
    diagnostics.begin();
    assert!(diagnostics.is_in_flight());

    diagnostics.abandon();

    assert_eq!(diagnostics.phase(), DiagnosticsPhase::NotRun);
    assert!(!diagnostics.is_in_flight());
    // The last-known-good report must survive being abandoned so a stale
    // probe result never clobbers it.
    assert_eq!(
        diagnostics.report().unwrap().active_account.as_deref(),
        Some("dev@example.com")
    );
}

#[test]
fn begin_after_abandon_re_arms_the_guard() {
    let mut diagnostics = OverviewDiagnostics::default();
    diagnostics.begin();
    diagnostics.abandon();

    diagnostics.begin();

    assert_eq!(diagnostics.phase(), DiagnosticsPhase::InFlight);
    assert!(diagnostics.is_in_flight());
}
