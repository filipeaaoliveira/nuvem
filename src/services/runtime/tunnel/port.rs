//! Local-port allocation for managed tunnels: deciding which `127.0.0.1` port
//! a new tunnel child should be told to bind.
//!
//! The probe is **advisory, not a reservation**. Every answer this module gives
//! is a snapshot: `is_free` binds a listener and immediately drops it, and
//! `ephemeral` asks the OS for a port and then releases it, so between the
//! probe and the tunnel child's own `bind()` another process (or another
//! tunnel started concurrently) can take the port. There is no way to hand a
//! bound socket to `gcloud`/`ssh`/`cloud-sql-proxy`, so that race is
//! irreducible. When it happens the child fails to bind and the loss surfaces
//! as the tunnel's `Failed` status carrying the child's own stderr — it is
//! deliberately *not* retried behind the user's back or hidden, because "the
//! port I asked for was taken" is exactly what the message says.
//!
//! `claimed` is the set of ports this app has already handed to live tunnels.
//! It is checked separately from `is_free` because a tunnel that is starting up
//! has not bound its port yet: the OS would still report it free, and two
//! tunnels would be pointed at the same port.

use std::net::{Ipv4Addr, TcpListener};

/// How far past a preferred port allocation will walk before giving up on the
/// neighbourhood and asking the OS for an ephemeral port. Small on purpose:
/// the point of walking up is that `5433` is still recognisably "the Postgres
/// one", which stops being true after a wide scan.
const WALK_UP_WINDOW: u16 = 16;

/// Probes local-port availability. A seam so allocation is unit-testable
/// without binding real sockets; the production implementation is
/// [`BindPortProbe`].
pub trait PortProbe: Send + Sync {
    /// Whether `port` can be bound on loopback right now. Advisory — see the
    /// module docs.
    fn is_free(&self, port: u16) -> bool;

    /// A port the OS considers free, or `None` when even that fails.
    fn ephemeral(&self) -> Option<u16>;
}

/// Production probe: answers by actually binding a loopback `TcpListener` and
/// dropping it again.
pub struct BindPortProbe;

impl PortProbe for BindPortProbe {
    fn is_free(&self, port: u16) -> bool {
        // Port 0 would make the OS pick one and always succeed, which would be
        // a false "free" answer for the literal port 0.
        port != 0 && TcpListener::bind((Ipv4Addr::LOCALHOST, port)).is_ok()
    }

    fn ephemeral(&self) -> Option<u16> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).ok()?;
        let port = listener.local_addr().ok()?.port();
        (port != 0).then_some(port)
    }
}

/// No local port could be allocated for a tunnel.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PortAllocationError {
    #[error("no local port is available for this tunnel")]
    NoPortAvailable,
}

/// Picks the local port a new tunnel should try to bind, in this order:
///
/// 1. `preferred`, when it is neither claimed by a live tunnel nor in use.
/// 2. the first free, unclaimed port in `preferred + 1 ..= preferred + 16`
///    (stopping early at `u16::MAX` rather than wrapping).
/// 3. an OS-assigned ephemeral port, retried once when the first one collides
///    with `claimed`.
/// 4. otherwise [`PortAllocationError::NoPortAvailable`].
///
/// Port `0` is never returned: to `bind()` it means "you pick", which is the
/// opposite of allocating a known port to show the user and point clients at.
pub fn allocate_local_port(
    preferred: Option<u16>,
    claimed: &[u16],
    probe: &dyn PortProbe,
) -> Result<u16, PortAllocationError> {
    if let Some(preferred) = preferred {
        if is_available(preferred, claimed, probe) {
            return Ok(preferred);
        }
        for offset in 1..=WALK_UP_WINDOW {
            // Ports above `u16::MAX` do not exist, so the walk stops short
            // rather than wrapping around into the reserved low ports.
            let Some(candidate) = preferred.checked_add(offset) else {
                break;
            };
            if is_available(candidate, claimed, probe) {
                return Ok(candidate);
            }
        }
    }

    // The OS picked this one, so it is free by construction; it only has to
    // clear `claimed`, which the OS knows nothing about. One retry: a second
    // collision means something is badly wrong, and looping would spin.
    for _ in 0..2 {
        match probe.ephemeral() {
            Some(port) if port != 0 && !claimed.contains(&port) => return Ok(port),
            _ => continue,
        }
    }

    Err(PortAllocationError::NoPortAvailable)
}

/// A port is usable when it is a real port, is not already promised to a live
/// tunnel, and the OS will let us bind it.
fn is_available(port: u16, claimed: &[u16], probe: &dyn PortProbe) -> bool {
    port != 0 && !claimed.contains(&port) && probe.is_free(port)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    /// Fake probe built from an explicit set of busy ports plus a scripted
    /// queue of `ephemeral()` results, so each allocation rule is provable in
    /// isolation without touching a real socket.
    struct FakeProbe {
        busy: Vec<u16>,
        ephemeral: Mutex<VecDeque<Option<u16>>>,
        is_free_calls: Mutex<Vec<u16>>,
    }

    impl FakeProbe {
        fn new(busy: &[u16], ephemeral: &[Option<u16>]) -> Self {
            Self {
                busy: busy.to_vec(),
                ephemeral: Mutex::new(ephemeral.iter().copied().collect()),
                is_free_calls: Mutex::new(Vec::new()),
            }
        }

        fn is_free_calls(&self) -> Vec<u16> {
            self.is_free_calls.lock().expect("lock").clone()
        }

        fn remaining_ephemeral(&self) -> usize {
            self.ephemeral.lock().expect("lock").len()
        }
    }

    impl PortProbe for FakeProbe {
        fn is_free(&self, port: u16) -> bool {
            self.is_free_calls.lock().expect("lock").push(port);
            !self.busy.contains(&port)
        }

        fn ephemeral(&self) -> Option<u16> {
            self.ephemeral.lock().expect("lock").pop_front().flatten()
        }
    }

    #[test]
    fn free_preferred_port_is_used_as_is() {
        let probe = FakeProbe::new(&[], &[Some(40000)]);
        assert_eq!(allocate_local_port(Some(5432), &[], &probe), Ok(5432));
        // The ephemeral fallback must not have been consulted at all.
        assert_eq!(probe.remaining_ephemeral(), 1);
    }

    #[test]
    fn preferred_port_claimed_by_a_live_tunnel_walks_up() {
        let probe = FakeProbe::new(&[], &[]);
        assert_eq!(allocate_local_port(Some(5432), &[5432], &probe), Ok(5433));
    }

    #[test]
    fn preferred_port_busy_at_the_os_walks_up() {
        let probe = FakeProbe::new(&[5432], &[]);
        assert_eq!(allocate_local_port(Some(5432), &[], &probe), Ok(5433));
    }

    #[test]
    fn walk_up_skips_several_busy_and_claimed_candidates() {
        let probe = FakeProbe::new(&[5432, 5433, 5435], &[]);
        assert_eq!(
            allocate_local_port(Some(5432), &[5434], &probe),
            Ok(5436),
            "5433/5435 busy at the OS, 5434 claimed by a live tunnel"
        );
    }

    #[test]
    fn walk_up_window_is_sixteen_ports_then_falls_through_to_ephemeral() {
        // 5432 plus the whole p+1..=p+16 window is busy; p+17 is free but out
        // of range, so allocation must fall through to the ephemeral probe.
        let busy: Vec<u16> = (5432..=5448).collect();
        let probe = FakeProbe::new(&busy, &[Some(41000)]);
        assert_eq!(allocate_local_port(Some(5432), &[], &probe), Ok(41000));
        assert_eq!(
            probe.is_free_calls().last().copied(),
            Some(5448),
            "the walk must stop at p+16, never reaching the free p+17"
        );
    }

    #[test]
    fn walk_up_saturates_at_u16_max_instead_of_wrapping_to_zero() {
        let probe = FakeProbe::new(&[u16::MAX], &[Some(41001)]);
        assert_eq!(allocate_local_port(Some(u16::MAX), &[], &probe), Ok(41001));
        assert_eq!(
            probe.is_free_calls(),
            vec![u16::MAX],
            "there is no port above u16::MAX, so the walk stops immediately"
        );
    }

    #[test]
    fn ephemeral_colliding_with_a_claimed_port_is_retried_once() {
        let probe = FakeProbe::new(&[], &[Some(41002), Some(41003)]);
        assert_eq!(
            allocate_local_port(None, &[41002], &probe),
            Ok(41003),
            "the first ephemeral port is claimed by a live tunnel"
        );
    }

    #[test]
    fn ephemeral_colliding_twice_yields_no_port_available() {
        let probe = FakeProbe::new(&[], &[Some(41004), Some(41004), Some(41005)]);
        assert_eq!(
            allocate_local_port(None, &[41004], &probe),
            Err(PortAllocationError::NoPortAvailable)
        );
        assert_eq!(
            probe.remaining_ephemeral(),
            1,
            "the ephemeral probe is retried exactly once, not in a loop"
        );
    }

    #[test]
    fn everything_unavailable_yields_no_port_available() {
        let busy: Vec<u16> = (5432..=5448).collect();
        let probe = FakeProbe::new(&busy, &[None, None]);
        assert_eq!(
            allocate_local_port(Some(5432), &[], &probe),
            Err(PortAllocationError::NoPortAvailable)
        );
    }

    #[test]
    fn no_preferred_port_goes_straight_to_ephemeral() {
        let probe = FakeProbe::new(&[], &[Some(41006)]);
        assert_eq!(allocate_local_port(None, &[], &probe), Ok(41006));
        assert!(
            probe.is_free_calls().is_empty(),
            "without a preferred port there is nothing to probe by bind"
        );
    }

    #[test]
    fn port_zero_is_never_returned() {
        // 0 means "let the OS pick" to `bind()`, so it must never be handed
        // back as an allocated port: a `Some(0)` preference walks up instead,
        // and a `Some(0)` ephemeral result is rejected.
        let probe = FakeProbe::new(&[], &[]);
        assert_eq!(allocate_local_port(Some(0), &[], &probe), Ok(1));

        let probe = FakeProbe::new(&[], &[Some(0), Some(41007)]);
        assert_eq!(allocate_local_port(None, &[], &probe), Ok(41007));
    }

    #[test]
    fn bind_port_probe_ephemeral_returns_a_nonzero_port() {
        let port = BindPortProbe
            .ephemeral()
            .expect("the OS should hand out an ephemeral loopback port");
        // Nonzero is the whole contract: 0 is what we *asked* the OS to pick
        // from, so reading the assigned port back is the point of the call.
        // Deliberately *not* asserted here: that `port` is still bindable
        // afterwards. It usually is, but the probe is advisory (see the module
        // docs) and any other process on the machine may take the port in that
        // window — asserting it made this test fail with EADDRINUSE.
        assert_ne!(port, 0);
    }

    #[test]
    fn bind_port_probe_reports_a_held_port_as_not_free() {
        // Deterministic negative: the listener stays alive for the probe, and
        // a second bind of the same loopback addr:port cannot succeed.
        let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .expect("bind an ephemeral loopback listener");
        let port = listener.local_addr().expect("local addr").port();
        assert!(!BindPortProbe.is_free(port));
        drop(listener);
    }

    #[test]
    fn bind_port_probe_never_reports_port_zero_as_free() {
        // Binding 0 always succeeds (the OS picks), which would be a false
        // "port 0 is free" answer, so the real probe short-circuits it.
        assert!(!BindPortProbe.is_free(0));
    }
}
