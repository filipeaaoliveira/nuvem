use nuvem::app::state::diagnostics::{DiagnosticsPhase, OverviewDiagnostics};
use nuvem_domain::gcp::diagnostics::GcloudDiagnosticsReport;

#[test]
fn default_is_not_run() {
    let diagnostics = OverviewDiagnostics::default();
    assert_eq!(diagnostics.phase(), DiagnosticsPhase::NotRun);
    assert!(!diagnostics.is_in_flight());
    assert!(diagnostics.report().is_none());
}

#[test]
fn begin_marks_in_flight() {
    let mut diagnostics = OverviewDiagnostics::default();
    diagnostics.begin();
    assert_eq!(diagnostics.phase(), DiagnosticsPhase::InFlight);
    assert!(diagnostics.is_in_flight());
}

#[test]
fn complete_stores_report() {
    let mut diagnostics = OverviewDiagnostics::default();
    diagnostics.begin();
    diagnostics.complete(GcloudDiagnosticsReport {
        available: true,
        active_account: Some("dev@example.com".to_string()),
        problem: None,
    });
    assert_eq!(diagnostics.phase(), DiagnosticsPhase::Complete);
    assert_eq!(
        diagnostics.report().unwrap().active_account.as_deref(),
        Some("dev@example.com")
    );
}
