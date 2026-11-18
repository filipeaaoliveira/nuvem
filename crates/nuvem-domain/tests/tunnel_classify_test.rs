//! Pins the Cloud SQL proxy readiness guard: only the documented readiness
//! phrase promotes a line to `Ready` — ordinary startup chatter is ignored.

use nuvem_domain::gcp::tunnel::{TunnelKind, TunnelSignal, classify_tunnel_line};

#[test]
fn proxy_chatter_without_the_readiness_phrase_is_ignored() {
    assert_eq!(
        classify_tunnel_line(
            TunnelKind::CloudSqlProxy,
            "2026/08/06 15:00:00 starting the proxy"
        ),
        TunnelSignal::Ignored
    );
}

#[test]
fn proxy_readiness_phrase_marks_the_tunnel_ready() {
    assert_eq!(
        classify_tunnel_line(
            TunnelKind::CloudSqlProxy,
            "2026/08/06 15:00:01 ready for new connections"
        ),
        TunnelSignal::Ready { local_port: None }
    );
}
