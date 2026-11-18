//! Pure planning for managed tunnels: pick the tunnel kind for a resource,
//! build the exact argv to spawn, decide the default ports, and classify a
//! line of child output.
//!
//! Everything a tunnel gets wrong happens out in the messy world — a process
//! that never comes up, a port already bound, gcloud changing its banner
//! wording. Keeping the decisions here, as total functions over plain inputs,
//! means the supervisor above only has to spawn a `TunnelSpec` and feed lines
//! back in: the fragile parts (argv shape, banner parsing, error detection)
//! are exercised by unit tests instead of by running real tunnels. Nothing in
//! this module touches the filesystem, the network, or `std::process`.

use crate::gcp::resource::ComputeInstance;
use crate::gcp::text::sanitize_display_text;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TunnelKind {
    IapTcp,
    SshLocalForward,
    CloudSqlProxy,
}

impl TunnelKind {
    /// Short badge the tunnel list renders next to each entry.
    pub fn label(self) -> &'static str {
        match self {
            Self::IapTcp => "IAP",
            Self::SshLocalForward => "SSH -L",
            Self::CloudSqlProxy => "SQL proxy",
        }
    }
}

/// tgcp's IAP auto-detection: an instance without a usable external IP is only
/// reachable through IAP, so it gets a `start-iap-tunnel`; anything with a
/// public address takes the cheaper plain `ssh -L`.
pub fn compute_tunnel_kind(instance: &ComputeInstance) -> TunnelKind {
    let has_external_ip = instance
        .external_ip
        .as_deref()
        .is_some_and(|ip| !ip.is_empty());
    if has_external_ip {
        TunnelKind::SshLocalForward
    } else {
        TunnelKind::IapTcp
    }
}

/// A tunnel's spawn, decided in full: the program, its argv, and the line the
/// UI shows for it.
///
/// Built here rather than at the spawn site because the confirmation preview
/// renders the same value that is later spawned, so what the operator approves
/// cannot drift from what runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TunnelSpec {
    pub program: String,
    pub args: Vec<String>,
    pub description: String,
}

/// `gcloud compute start-iap-tunnel` pinned to loopback: `--local-host-port`
/// keeps the listener on `localhost` so the forward is never world-reachable.
pub fn iap_tunnel_spec(
    gcloud_binary: &str,
    configuration: &str,
    instance_name: &str,
    zone: &str,
    remote_port: u16,
    local_port: u16,
) -> TunnelSpec {
    TunnelSpec {
        program: gcloud_binary.to_string(),
        args: vec![
            "--configuration".into(),
            configuration.to_string(),
            "compute".into(),
            "start-iap-tunnel".into(),
            instance_name.to_string(),
            remote_port.to_string(),
            format!("--local-host-port=localhost:{local_port}"),
            "--zone".into(),
            zone.to_string(),
        ],
        description: format!("IAP tunnel to {instance_name}:{remote_port}"),
    }
}

/// `gcloud compute ssh -- -N -o ExitOnForwardFailure=yes -L …`. `-N` requests
/// no remote command, so the child stays a silent forwarder instead of opening
/// an interactive shell that would fight the TUI for the terminal.
///
/// `ExitOnForwardFailure=yes` is what makes a forward that cannot bind actually
/// end. By default `ssh` prints `bind [127.0.0.1]:5432: Address already in use`
/// and keeps running, so the row would sit `Active` forever on a forward that
/// carries nothing.
pub fn ssh_local_forward_spec(
    gcloud_binary: &str,
    configuration: &str,
    instance_name: &str,
    zone: &str,
    local_port: u16,
    remote_port: u16,
) -> TunnelSpec {
    TunnelSpec {
        program: gcloud_binary.to_string(),
        args: vec![
            "--configuration".into(),
            configuration.to_string(),
            "compute".into(),
            "ssh".into(),
            instance_name.to_string(),
            "--zone".into(),
            zone.to_string(),
            "--".into(),
            "-N".into(),
            "-o".into(),
            "ExitOnForwardFailure=yes".into(),
            "-L".into(),
            format!("{local_port}:localhost:{remote_port}"),
        ],
        description: format!("SSH forward to {instance_name}:{remote_port}"),
    }
}

/// The `project:region:instance` triple the Cloud SQL Auth Proxy expects.
pub fn cloud_sql_connection_name(project: &str, region: &str, instance: &str) -> String {
    format!("{project}:{region}:{instance}")
}

/// Cloud SQL Auth Proxy bound explicitly to `127.0.0.1` — the proxy speaks
/// unauthenticated on its local socket, so it must never listen on `0.0.0.0`.
pub fn cloud_sql_proxy_spec(
    proxy_binary: &str,
    connection_name: &str,
    local_port: u16,
) -> TunnelSpec {
    TunnelSpec {
        program: proxy_binary.to_string(),
        args: vec![
            "--address".into(),
            "127.0.0.1".into(),
            "--port".into(),
            local_port.to_string(),
            connection_name.to_string(),
        ],
        description: format!("Cloud SQL proxy to {connection_name}"),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TunnelReadiness {
    AwaitSignal,
    OnSpawn,
}

/// Wire port for a Cloud SQL engine. Accepts both the normalized
/// `CloudSqlInstance::engine` labels (`"PostgreSQL"`) and the raw
/// `database_version` strings (`"POSTGRES_15"`) because callers hold whichever
/// one their list row happened to carry; matching is case-insensitive and
/// space-insensitive so `"SQL Server"` and `"SQLSERVER_2019_STANDARD"` agree.
fn engine_port(engine: Option<&str>) -> Option<u16> {
    let engine = engine?.trim().to_ascii_lowercase().replace(' ', "");
    if engine.starts_with("postgres") {
        Some(5432)
    } else if engine.starts_with("mysql") {
        Some(3306)
    } else if engine.starts_with("sqlserver") {
        Some(1433)
    } else {
        None
    }
}

/// Preferred local listen port. Instance tunnels return `None` so the caller
/// can let the OS pick an ephemeral port; only the SQL proxy has a
/// conventional port worth defaulting to.
pub fn default_local_port(kind: TunnelKind, engine: Option<&str>) -> Option<u16> {
    match kind {
        TunnelKind::CloudSqlProxy => engine_port(engine),
        TunnelKind::IapTcp | TunnelKind::SshLocalForward => None,
    }
}

/// Port on the far side of the tunnel: sshd for instance tunnels, the engine's
/// wire port for the SQL proxy (PostgreSQL when the engine is unknown, since
/// that is the most common Cloud SQL flavour).
pub fn default_remote_port(kind: TunnelKind, engine: Option<&str>) -> u16 {
    match kind {
        TunnelKind::IapTcp | TunnelKind::SshLocalForward => 22,
        TunnelKind::CloudSqlProxy => engine_port(engine).unwrap_or(5432),
    }
}

/// When a tunnel may be reported as up. IAP and the SQL proxy print a banner
/// once they are listening, so we wait for it; `ssh -N` prints nothing at all
/// on success, so a healthy spawn is the only signal we will ever get.
pub fn readiness(kind: TunnelKind) -> TunnelReadiness {
    match kind {
        TunnelKind::IapTcp | TunnelKind::CloudSqlProxy => TunnelReadiness::AwaitSignal,
        TunnelKind::SshLocalForward => TunnelReadiness::OnSpawn,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TunnelSignal {
    Ready { local_port: Option<u16> },
    Failed { message: String },
    Ignored,
}

/// Substrings that mean the tunnel **cannot carry traffic**, whenever they
/// appear — before readiness or long after it. Each one names a condition no
/// running child recovers from on its own: the local listener could not be
/// bound, or the far side refused us outright.
///
/// Deliberately narrow. This is the set a *ready* tunnel is still killed for,
/// so a marker that can also appear on a healthy child (a bare `error`, a
/// per-connection `failed`) does not belong here — see [`BROAD_ERROR_MARKERS`].
const ALWAYS_FATAL_MARKERS: [&str; 8] = [
    // `ssh`'s own bind failure: `bind [127.0.0.1]:5432: Address already in use`.
    "bind [",
    // Go's: `listen tcp 127.0.0.1:5432: bind: Address already in use`.
    "bind: ",
    "address already in use",
    "permission denied",
    "denied by iap",
    "host key verification failed",
    "authentication failed",
    "authentication failure",
];

/// Substrings that only *look* alarming. They are what stops a drifting startup
/// banner from hanging a tunnel in `Starting` forever, so they are fatal before
/// readiness — but a working `ssh -L` prints
/// `channel 3: open failed: connect failed: Connection refused` for one refused
/// forwarded connection, and `gcloud` prints `...failed.` spinner text, so
/// acting on them afterwards would tear down a healthy tunnel.
const BROAD_ERROR_MARKERS: [&str; 2] = ["error", "failed"];

/// Whether this line is fatal no matter when it arrives.
///
/// A pure "what does this line mean?" predicate: the *policy* of what to do
/// with a line lives in the supervisor, which fails a tunnel on any error line
/// before readiness and on these lines only once it is up.
pub fn is_always_fatal_tunnel_line(line: &str) -> bool {
    let lowered = line.to_lowercase();
    ALWAYS_FATAL_MARKERS
        .iter()
        .any(|marker| lowered.contains(marker))
}

/// Longest failure message we keep, in characters. Child output is untrusted
/// and unbounded; the status line only has room for a sentence.
const MAX_FAILURE_MESSAGE_CHARS: usize = 200;

/// Pulls the port out of gcloud's `Listening on port [8022].` banner.
fn bracketed_port(line: &str) -> Option<u16> {
    let start = line.find('[')?;
    let rest = &line[start + 1..];
    let end = rest.find(']')?;
    rest[..end].trim().parse().ok()
}

/// Reads one line of child output. Failure detection runs before readiness so
/// a line that mentions both (`ERROR: ... Listening on port [8022] ...`) never
/// marks a dead tunnel as up.
pub fn classify_tunnel_line(kind: TunnelKind, line: &str) -> TunnelSignal {
    if line.trim().is_empty() {
        return TunnelSignal::Ignored;
    }

    let lowered = line.to_lowercase();
    let looks_like_a_failure = ALWAYS_FATAL_MARKERS
        .iter()
        .chain(BROAD_ERROR_MARKERS.iter())
        .any(|marker| lowered.contains(marker));
    if looks_like_a_failure {
        let message = sanitize_display_text(line)
            .chars()
            .take(MAX_FAILURE_MESSAGE_CHARS)
            .collect();
        return TunnelSignal::Failed { message };
    }

    match kind {
        TunnelKind::IapTcp if lowered.contains("listening on port") => TunnelSignal::Ready {
            local_port: bracketed_port(line),
        },
        TunnelKind::CloudSqlProxy if lowered.contains("ready for new connections") => {
            TunnelSignal::Ready { local_port: None }
        }
        _ => TunnelSignal::Ignored,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gcp::resource::{ComputeInstance, ResourceStatus};

    fn instance(external_ip: Option<&str>) -> ComputeInstance {
        ComputeInstance {
            id: "1".into(),
            name: "web-1".into(),
            zone: "us-central1-a".into(),
            status: ResourceStatus::Running,
            machine_type: "e2-medium".into(),
            internal_ip: Some("10.0.0.2".into()),
            external_ip: external_ip.map(Into::into),
        }
    }

    #[test]
    fn kind_is_iap_when_instance_has_no_external_ip() {
        assert_eq!(compute_tunnel_kind(&instance(None)), TunnelKind::IapTcp);
    }

    #[test]
    fn kind_is_ssh_when_instance_has_external_ip() {
        assert_eq!(
            compute_tunnel_kind(&instance(Some("34.1.2.3"))),
            TunnelKind::SshLocalForward
        );
    }

    #[test]
    fn empty_external_ip_counts_as_absent() {
        assert_eq!(compute_tunnel_kind(&instance(Some(""))), TunnelKind::IapTcp);
    }

    #[test]
    fn iap_spec_builds_start_iap_tunnel_argv() {
        let spec = iap_tunnel_spec("gcloud", "dev", "web-1", "us-central1-a", 22, 8022);
        assert_eq!(spec.program, "gcloud");
        assert_eq!(
            spec.args,
            vec![
                "--configuration",
                "dev",
                "compute",
                "start-iap-tunnel",
                "web-1",
                "22",
                "--local-host-port=localhost:8022",
                "--zone",
                "us-central1-a",
            ]
        );
        assert_eq!(spec.description, "IAP tunnel to web-1:22");
    }

    #[test]
    fn ssh_spec_builds_no_command_local_forward_argv() {
        let spec = ssh_local_forward_spec("gcloud", "dev", "web-1", "us-central1-a", 9200, 9200);
        assert_eq!(spec.program, "gcloud");
        assert_eq!(
            spec.args,
            vec![
                "--configuration",
                "dev",
                "compute",
                "ssh",
                "web-1",
                "--zone",
                "us-central1-a",
                "--",
                "-N",
                // Without this `ssh` reports the bind failure and keeps
                // running, leaving an `Active` row on a forward that carries
                // nothing.
                "-o",
                "ExitOnForwardFailure=yes",
                "-L",
                "9200:localhost:9200",
            ]
        );
        assert_eq!(spec.description, "SSH forward to web-1:9200");
    }

    #[test]
    fn cloud_sql_connection_name_joins_with_colons() {
        assert_eq!(
            cloud_sql_connection_name("proj", "us-central1", "db-1"),
            "proj:us-central1:db-1"
        );
    }

    #[test]
    fn cloud_sql_proxy_spec_binds_loopback_only() {
        let spec = cloud_sql_proxy_spec("cloud-sql-proxy", "proj:us-central1:db-1", 5432);
        assert_eq!(spec.program, "cloud-sql-proxy");
        assert_eq!(
            spec.args,
            vec![
                "--address",
                "127.0.0.1",
                "--port",
                "5432",
                "proj:us-central1:db-1",
            ]
        );
        assert_eq!(spec.description, "Cloud SQL proxy to proj:us-central1:db-1");
    }

    #[test]
    fn kind_labels_are_the_view_strings() {
        assert_eq!(TunnelKind::IapTcp.label(), "IAP");
        assert_eq!(TunnelKind::SshLocalForward.label(), "SSH -L");
        assert_eq!(TunnelKind::CloudSqlProxy.label(), "SQL proxy");
    }

    #[test]
    fn default_local_port_from_normalized_engine_labels() {
        assert_eq!(
            default_local_port(TunnelKind::CloudSqlProxy, Some("PostgreSQL")),
            Some(5432)
        );
        assert_eq!(
            default_local_port(TunnelKind::CloudSqlProxy, Some("MySQL")),
            Some(3306)
        );
        assert_eq!(
            default_local_port(TunnelKind::CloudSqlProxy, Some("SQL Server")),
            Some(1433)
        );
    }

    #[test]
    fn default_local_port_from_raw_database_version_strings() {
        assert_eq!(
            default_local_port(TunnelKind::CloudSqlProxy, Some("POSTGRES_15")),
            Some(5432)
        );
        assert_eq!(
            default_local_port(TunnelKind::CloudSqlProxy, Some("MYSQL_8_0")),
            Some(3306)
        );
        assert_eq!(
            default_local_port(TunnelKind::CloudSqlProxy, Some("SQLSERVER_2019_STANDARD")),
            Some(1433)
        );
    }

    #[test]
    fn default_local_port_is_none_for_unknown_engine_and_ssh_kinds() {
        assert_eq!(
            default_local_port(TunnelKind::CloudSqlProxy, Some("cassandra")),
            None
        );
        assert_eq!(default_local_port(TunnelKind::CloudSqlProxy, None), None);
        assert_eq!(default_local_port(TunnelKind::IapTcp, None), None);
        assert_eq!(
            default_local_port(TunnelKind::SshLocalForward, Some("PostgreSQL")),
            None
        );
    }

    #[test]
    fn default_remote_port_is_ssh_for_instance_tunnels() {
        assert_eq!(default_remote_port(TunnelKind::IapTcp, None), 22);
        assert_eq!(default_remote_port(TunnelKind::SshLocalForward, None), 22);
        assert_eq!(
            default_remote_port(TunnelKind::IapTcp, Some("POSTGRES_15")),
            22
        );
    }

    #[test]
    fn default_remote_port_for_proxy_follows_engine_and_falls_back_to_postgres() {
        assert_eq!(
            default_remote_port(TunnelKind::CloudSqlProxy, Some("MYSQL_8_0")),
            3306
        );
        assert_eq!(
            default_remote_port(TunnelKind::CloudSqlProxy, Some("SQL Server")),
            1433
        );
        assert_eq!(default_remote_port(TunnelKind::CloudSqlProxy, None), 5432);
        assert_eq!(
            default_remote_port(TunnelKind::CloudSqlProxy, Some("cassandra")),
            5432
        );
    }

    #[test]
    fn engine_matching_is_case_insensitive() {
        assert_eq!(
            default_local_port(TunnelKind::CloudSqlProxy, Some("postgres_15")),
            Some(5432)
        );
        assert_eq!(
            default_local_port(TunnelKind::CloudSqlProxy, Some("sqlserver_2019_web")),
            Some(1433)
        );
    }

    #[test]
    fn readiness_awaits_a_banner_except_for_silent_ssh() {
        assert_eq!(readiness(TunnelKind::IapTcp), TunnelReadiness::AwaitSignal);
        assert_eq!(
            readiness(TunnelKind::CloudSqlProxy),
            TunnelReadiness::AwaitSignal
        );
        assert_eq!(
            readiness(TunnelKind::SshLocalForward),
            TunnelReadiness::OnSpawn
        );
    }

    #[test]
    fn iap_listening_banner_yields_ready_with_parsed_port() {
        assert_eq!(
            classify_tunnel_line(TunnelKind::IapTcp, "Listening on port [8022]."),
            TunnelSignal::Ready {
                local_port: Some(8022)
            }
        );
    }

    #[test]
    fn iap_listening_banner_without_parsable_port_still_reports_ready() {
        assert_eq!(
            classify_tunnel_line(TunnelKind::IapTcp, "Listening on port [????]."),
            TunnelSignal::Ready { local_port: None }
        );
        assert_eq!(
            classify_tunnel_line(TunnelKind::IapTcp, "Listening on port."),
            TunnelSignal::Ready { local_port: None }
        );
    }

    #[test]
    fn proxy_ready_banner_yields_ready_without_a_port() {
        assert_eq!(
            classify_tunnel_line(
                TunnelKind::CloudSqlProxy,
                "2026/07/22 13:00:00 The proxy has started successfully and is ready for new connections!"
            ),
            TunnelSignal::Ready { local_port: None }
        );
    }

    #[test]
    fn address_already_in_use_is_a_failure() {
        assert_eq!(
            classify_tunnel_line(
                TunnelKind::CloudSqlProxy,
                "listen tcp 127.0.0.1:5432: bind: Address already in use"
            ),
            TunnelSignal::Failed {
                message: "listen tcp 127.0.0.1:5432: bind: Address already in use".into()
            }
        );
    }

    #[test]
    fn error_wins_over_a_ready_banner_on_the_same_line() {
        assert_eq!(
            classify_tunnel_line(
                TunnelKind::IapTcp,
                "ERROR: Listening on port [8022]. failed to establish connection"
            ),
            TunnelSignal::Failed {
                message: "ERROR: Listening on port [8022]. failed to establish connection".into()
            }
        );
    }

    #[test]
    fn failure_message_is_sanitized() {
        assert_eq!(
            classify_tunnel_line(TunnelKind::IapTcp, "ERROR: denied by IAP\u{7}\u{1b}[2J"),
            TunnelSignal::Failed {
                message: "ERROR: denied by IAP\u{fffd}\u{fffd}[2J".into()
            }
        );
    }

    #[test]
    fn failure_message_is_truncated_to_two_hundred_chars() {
        let line = format!("ERROR: {}", "é".repeat(500));
        let TunnelSignal::Failed { message } = classify_tunnel_line(TunnelKind::IapTcp, &line)
        else {
            panic!("expected a failure signal");
        };
        assert_eq!(message.chars().count(), 200);
        assert!(message.starts_with("ERROR: ééé"));
    }

    #[test]
    fn blank_and_unremarkable_lines_are_ignored() {
        assert_eq!(
            classify_tunnel_line(TunnelKind::IapTcp, ""),
            TunnelSignal::Ignored
        );
        assert_eq!(
            classify_tunnel_line(TunnelKind::IapTcp, "   "),
            TunnelSignal::Ignored
        );
        assert_eq!(
            classify_tunnel_line(TunnelKind::IapTcp, "Testing if tunnel connection works."),
            TunnelSignal::Ignored
        );
        assert_eq!(
            classify_tunnel_line(TunnelKind::SshLocalForward, "Warm-up complete"),
            TunnelSignal::Ignored
        );
    }

    #[test]
    fn ssh_forward_never_reports_ready_from_output() {
        assert_eq!(
            classify_tunnel_line(TunnelKind::SshLocalForward, "Listening on port [8022]."),
            TunnelSignal::Ignored
        );
    }

    /// Tier one of the post-readiness policy: lines that mean the tunnel
    /// cannot carry traffic, whenever they appear.
    #[test]
    fn lines_that_mean_no_traffic_are_always_fatal() {
        for line in [
            "bind [127.0.0.1]:5432: Address already in use",
            "listen tcp 127.0.0.1:5432: bind: Address already in use",
            "bind: cannot assign requested address",
            "Permission denied (publickey).",
            "ERROR: (gcloud.compute.start-iap-tunnel) Connection denied by IAP",
            "Host key verification failed.",
            "Authentication failed.",
            "Received disconnect: Too many authentication failures",
        ] {
            assert!(
                is_always_fatal_tunnel_line(line),
                "expected {line:?} to be always fatal"
            );
        }
    }

    /// Tier two: lines the broad markers catch, which are ordinary chatter from
    /// a working child once it is up.
    #[test]
    fn alarming_but_benign_lines_are_not_always_fatal() {
        for line in [
            "channel 3: open failed: connect failed: Connection refused",
            "ERROR: something transient happened",
            "Testing if tunnel connection works.",
            "gcloud: refresh failed, retrying",
        ] {
            assert!(
                !is_always_fatal_tunnel_line(line),
                "expected {line:?} not to be always fatal"
            );
        }
    }

    #[test]
    fn always_fatal_matching_is_case_insensitive() {
        assert!(is_always_fatal_tunnel_line(
            "BIND [127.0.0.1]:5432: ADDRESS ALREADY IN USE"
        ));
        assert!(is_always_fatal_tunnel_line("HOST KEY VERIFICATION FAILED"));
    }

    #[test]
    fn error_markers_match_case_insensitively() {
        for line in [
            "something Failed",
            "Permission Denied (publickey)",
            "Denied by IAP",
            "bind: cannot assign requested address",
        ] {
            assert!(
                matches!(
                    classify_tunnel_line(TunnelKind::SshLocalForward, line),
                    TunnelSignal::Failed { .. }
                ),
                "expected {line:?} to classify as a failure"
            );
        }
    }
}
