use nuvem_domain::gcp::diagnostics::GcloudDiagnosticsReport;

/// Lifecycle of the targeted gcloud availability/auth probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DiagnosticsPhase {
    #[default]
    NotRun,
    InFlight,
    Complete,
}

/// Overview diagnostics state: probe lifecycle plus the last probe report.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OverviewDiagnostics {
    phase: DiagnosticsPhase,
    report: Option<GcloudDiagnosticsReport>,
}

impl OverviewDiagnostics {
    pub fn phase(&self) -> DiagnosticsPhase {
        self.phase
    }

    pub fn is_in_flight(&self) -> bool {
        self.phase == DiagnosticsPhase::InFlight
    }

    /// Marks a probe as in flight (used by the reducer as a no-refanout guard).
    pub fn begin(&mut self) {
        self.phase = DiagnosticsPhase::InFlight;
    }

    /// Records a completed probe report.
    pub fn complete(&mut self, report: GcloudDiagnosticsReport) {
        self.phase = DiagnosticsPhase::Complete;
        self.report = Some(report);
    }

    /// Releases the in-flight guard without touching the stored report.
    ///
    /// Used when a probe result arrives for a context generation that no
    /// longer matches the current one: the result itself is stale and must
    /// be dropped, but the `is_in_flight()` guard still has to be freed so
    /// the next probe can run. A stale probe must never overwrite a good
    /// report, so `report` is left untouched.
    pub fn abandon(&mut self) {
        self.phase = DiagnosticsPhase::NotRun;
    }

    pub fn report(&self) -> Option<&GcloudDiagnosticsReport> {
        self.report.as_ref()
    }
}
