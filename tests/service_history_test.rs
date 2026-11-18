//! The bounded, context-stamped per-service health history.
//!
//! This slice is pure state — nothing here renders. The properties it has to
//! hold are the three that make a sparkline honest rather than merely present:
//! samples are bounded, they are stamped with the context they were taken in,
//! and they are appended when a listing *result* lands rather than when a poll
//! *ticks*.

use std::time::{Duration, Instant};

use nuvem::app::state::health::{
    HISTORY_CAPACITY, HISTORY_CONTEXTS, ServiceHealthHistory, observe_services,
};
use nuvem::app::state::resources::ResourcesState;
use nuvem_domain::gcp::configuration::{GcloudConfigurationName, GcloudContext};
use nuvem_domain::gcp::resource::{
    ComputeInstance, ResourceServiceKind, ResourceStatus, StorageBucket,
};

fn context(name: &str) -> GcloudContext {
    GcloudContext {
        configuration: GcloudConfigurationName::new(name),
        account: None,
        project: Some(format!("{name}-project")),
        region: None,
        zone: None,
    }
}

fn instance(name: &str) -> ComputeInstance {
    ComputeInstance {
        id: name.to_string(),
        name: name.to_string(),
        zone: "europe-west1-b".to_string(),
        status: ResourceStatus::Running,
        machine_type: "e2-medium".to_string(),
        internal_ip: None,
        external_ip: None,
    }
}

fn bucket(name: &str) -> StorageBucket {
    StorageBucket {
        name: name.to_string(),
        location: "EU".to_string(),
        location_type: None,
        storage_class: "STANDARD".to_string(),
        versioning_enabled: false,
        uniform_bucket_level_access: true,
        public_access_prevention: None,
        time_created: None,
        updated: None,
    }
}

/// Lands a compute listing of `count` rows in `context`, the way a completed
/// poll does: a load is begun, and then its result is applied.
fn land_compute_listing(state: &mut ResourcesState, ctx: &GcloudContext, count: usize) {
    state.begin_load(ResourceServiceKind::ComputeInstances, Instant::now());
    let rows = (0..count).map(|i| instance(&format!("vm-{i}"))).collect();
    state.set_instances(ctx.clone(), rows);
}

#[test]
fn a_landed_listing_result_is_observed_with_its_row_count_and_context() {
    let ctx = context("alpha");
    let mut state = ResourcesState::default();
    land_compute_listing(&mut state, &ctx, 3);

    let observations = observe_services(&state);

    let compute: Vec<_> = observations
        .iter()
        .filter(|observation| observation.kind == ResourceServiceKind::ComputeInstances)
        .collect();
    assert_eq!(compute.len(), 1, "one landed result is one observation");
    assert_eq!(compute[0].total, 3);
    assert_eq!(compute[0].context, ctx);
}

/// The sample is the *result*, not the tick. A poll that is still in flight has
/// produced no count, and the rows still held are the previous result's — which
/// this observation would otherwise re-record under the new load's identity,
/// hiding the real one when it lands.
#[test]
fn a_service_whose_load_is_still_in_flight_is_not_observed() {
    for kind in ResourceServiceKind::ALL {
        let mut state = ResourcesState::default();
        state.begin_load(kind, Instant::now());

        let observed = observe_services(&state)
            .into_iter()
            .any(|observation| observation.kind == kind);

        assert!(
            !observed,
            "{kind:?} was observed while its load was in flight"
        );
    }
}

/// A failed poll produced no count. Recording a zero would paint an outage as a
/// drop to zero, which is a different and far more alarming claim.
#[test]
fn a_service_whose_load_failed_is_not_observed() {
    for kind in ResourceServiceKind::ALL {
        let mut state = ResourcesState::default();
        state.begin_load(kind, Instant::now());
        state.fail_load(kind, context("alpha"), "boom".to_string());

        let observed = observe_services(&state)
            .into_iter()
            .any(|observation| observation.kind == kind);

        assert!(!observed, "{kind:?} was observed after its load failed");
    }
}

#[test]
fn a_service_that_has_never_loaded_is_not_observed() {
    let state = ResourcesState::default();

    assert!(observe_services(&state).is_empty());
}

#[test]
fn recording_an_observation_appends_one_sample_to_that_services_series() {
    let ctx = context("alpha");
    let mut state = ResourcesState::default();
    land_compute_listing(&mut state, &ctx, 3);
    let mut history = ServiceHealthHistory::default();

    for observation in observe_services(&state) {
        history.record(observation, Instant::now());
    }

    assert_eq!(
        history.series_in(ResourceServiceKind::ComputeInstances, &ctx),
        vec![3]
    );
}

/// The sampler runs on every reduction, not once per landed result, so the same
/// settled listing is offered to `record` again and again. Only the first
/// offer of a given load may become a sample; otherwise a busy UI would fill
/// the ring with copies of one poll and the sparkline would read as a long
/// steady run that never happened.
#[test]
fn re_offering_the_same_landed_load_does_not_append_a_second_sample() {
    let ctx = context("alpha");
    let mut state = ResourcesState::default();
    land_compute_listing(&mut state, &ctx, 3);
    let mut history = ServiceHealthHistory::default();

    let appended: Vec<bool> = (0..4)
        .map(|_| {
            // Folded rather than `any`, which would short-circuit and leave
            // later services unrecorded.
            observe_services(&state)
                .into_iter()
                .fold(false, |appended, observation| {
                    history.record(observation, Instant::now()) || appended
                })
        })
        .collect();

    assert_eq!(appended, vec![true, false, false, false]);
    assert_eq!(
        history.series_in(ResourceServiceKind::ComputeInstances, &ctx),
        vec![3]
    );
}

#[test]
fn a_second_landed_load_appends_a_second_sample_even_at_the_same_count() {
    let ctx = context("alpha");
    let mut state = ResourcesState::default();
    let mut history = ServiceHealthHistory::default();

    for _ in 0..3 {
        land_compute_listing(&mut state, &ctx, 3);
        for observation in observe_services(&state) {
            history.record(observation, Instant::now());
        }
    }

    assert_eq!(
        history.series_in(ResourceServiceKind::ComputeInstances, &ctx),
        vec![3, 3, 3],
        "a re-list returning the same count is a real sample, and a flat run is real data"
    );
}

/// Bounded: a session left open for a day at a 30s poll must not grow. The
/// capacity is the sparkline's width budget, not "as much as we can hold".
#[test]
fn the_ring_evicts_the_oldest_sample_once_it_is_full() {
    let ctx = context("alpha");
    let mut state = ResourcesState::default();
    let mut history = ServiceHealthHistory::default();

    for count in 0..HISTORY_CAPACITY + 5 {
        land_compute_listing(&mut state, &ctx, count);
        for observation in observe_services(&state) {
            history.record(observation, Instant::now());
        }
    }

    let series = history.series_in(ResourceServiceKind::ComputeInstances, &ctx);
    assert_eq!(series.len(), HISTORY_CAPACITY);
    assert_eq!(
        series.first().copied(),
        Some(5),
        "the oldest five samples must have been evicted, not the newest"
    );
    assert_eq!(series.last().copied(), Some(HISTORY_CAPACITY + 4));
}

/// The hazard the whole design turns on: nuvem switches configuration and
/// project underneath a live session. A ring that did not filter on the active
/// context would paint project alpha's history under project beta's label, and
/// a sparkline is the one widget where wrong data looks exactly like right
/// data.
#[test]
fn a_series_holds_only_the_samples_taken_in_the_context_it_is_read_for() {
    let alpha = context("alpha");
    let beta = context("beta");
    let mut state = ResourcesState::default();
    let mut history = ServiceHealthHistory::default();

    for count in [1, 2] {
        land_compute_listing(&mut state, &alpha, count);
        for observation in observe_services(&state) {
            history.record(observation, Instant::now());
        }
    }
    for count in [7, 8] {
        land_compute_listing(&mut state, &beta, count);
        for observation in observe_services(&state) {
            history.record(observation, Instant::now());
        }
    }

    assert_eq!(
        history.series_in(ResourceServiceKind::ComputeInstances, &alpha),
        vec![1, 2]
    );
    assert_eq!(
        history.series_in(ResourceServiceKind::ComputeInstances, &beta),
        vec![7, 8]
    );
}

/// The context stamp is the whole context, not the configuration name. A
/// configuration's project changes underneath a stable name — through the
/// context edit form, or a `gcloud config set project` picked up by a reload —
/// and a name-only stamp would carry the old project's counts across.
#[test]
fn a_project_change_under_a_stable_configuration_name_starts_a_new_series() {
    let before = GcloudContext {
        project: Some("before".to_string()),
        ..context("alpha")
    };
    let after = GcloudContext {
        project: Some("after".to_string()),
        ..context("alpha")
    };
    let mut state = ResourcesState::default();
    let mut history = ServiceHealthHistory::default();

    land_compute_listing(&mut state, &before, 4);
    for observation in observe_services(&state) {
        history.record(observation, Instant::now());
    }
    land_compute_listing(&mut state, &after, 9);
    for observation in observe_services(&state) {
        history.record(observation, Instant::now());
    }

    assert_eq!(
        history.series_in(ResourceServiceKind::ComputeInstances, &before),
        vec![4]
    );
    assert_eq!(
        history.series_in(ResourceServiceKind::ComputeInstances, &after),
        vec![9]
    );
}

/// A filter is pushed down to `gcloud` as `--filter=<expression>`, so the rows
/// a filtered listing lands are a *subset* of the service's population. Sampled
/// into the same series as the unfiltered ones, typing a filter would draw a
/// cliff and clearing it a recovery, neither of which happened to anything in
/// the project.
///
/// This is the context stamp's failure one axis over, and the test is written
/// the same way: the count only means "how many of these exist" while nothing
/// is narrowing it.
#[test]
fn a_service_whose_filter_is_applied_is_not_observed() {
    let ctx = context("alpha");
    let mut state = ResourcesState::default();
    land_compute_listing(&mut state, &ctx, 40);
    assert!(
        observe_services(&state)
            .iter()
            .any(|observation| observation.kind == ResourceServiceKind::ComputeInstances),
        "the unfiltered listing is the observable one"
    );

    state.select_service(ResourceServiceKind::ComputeInstances);
    for character in "name~web".chars() {
        state.append_filter_char(character);
    }
    state.apply_filter();
    land_compute_listing(&mut state, &ctx, 6);

    let observed = observe_services(&state)
        .into_iter()
        .any(|observation| observation.kind == ResourceServiceKind::ComputeInstances);

    assert!(
        !observed,
        "a filtered listing is a subset, not a population, and must not become a sample"
    );
}

/// Clearing the filter makes the service observable again — the rule is about
/// what the rows currently mean, not a latch that a filtered listing trips
/// forever.
#[test]
fn clearing_the_filter_makes_the_service_observable_again() {
    let ctx = context("alpha");
    let mut state = ResourcesState::default();
    state.select_service(ResourceServiceKind::ComputeInstances);
    for character in "name~web".chars() {
        state.append_filter_char(character);
    }
    state.apply_filter();
    land_compute_listing(&mut state, &ctx, 6);

    for _ in 0.."name~web".len() {
        state.backspace_filter();
    }
    state.apply_filter();
    land_compute_listing(&mut state, &ctx, 40);

    let compute: Vec<_> = observe_services(&state)
        .into_iter()
        .filter(|observation| observation.kind == ResourceServiceKind::ComputeInstances)
        .collect();
    assert_eq!(compute.len(), 1);
    assert_eq!(compute[0].total, 40);
}

/// A load the runtime's generation guard threw away must not become a sample.
///
/// `abandon_load` resolves the phase to `Idle` under a sequence that never
/// landed, while the rows and the context stamp still belong to the *previous*
/// result. Keyed on the load's own sequence, the sampler would read that as a
/// fresh observation and append the previous count again, stamped with the time
/// of a fetch that was discarded — which is both the "long steady run that
/// never happened" the dedupe guard exists to prevent and, once the delta
/// window reads these stamps, a cell flashing over nothing.
#[test]
fn a_load_the_runtime_abandoned_does_not_become_a_sample() {
    let ctx = context("alpha");
    let mut state = ResourcesState::default();
    let mut history = ServiceHealthHistory::default();
    land_compute_listing(&mut state, &ctx, 4);
    for observation in observe_services(&state) {
        history.record(observation, Instant::now());
    }

    // A second fetch is dispatched and then dropped by the scope-generation
    // guard, which resolves the phase without any rows having landed.
    let ticket = state.begin_load(ResourceServiceKind::ComputeInstances, Instant::now());
    assert!(
        state.abandon_load(ticket),
        "the abandon must be the call that resolved the phase"
    );

    let appended = observe_services(&state)
        .into_iter()
        .fold(false, |appended, observation| {
            history.record(observation, Instant::now()) || appended
        });

    assert!(!appended, "a discarded fetch produced no count to record");
    assert_eq!(
        history.series_in(ResourceServiceKind::ComputeInstances, &ctx),
        vec![4]
    );
}

/// The capacity is per *series*, and a series is read per (service, context).
///
/// A ring shared across contexts would give an operator alternating two
/// configurations half a sparkline each, and thirty-two samples in one context
/// would evict another's history entirely — leaving a service that has been
/// polled all day rendering as one never listed.
#[test]
fn each_context_gets_its_own_full_capacity() {
    let alpha = context("alpha");
    let beta = context("beta");
    let mut state = ResourcesState::default();
    let mut history = ServiceHealthHistory::default();

    for count in 0..HISTORY_CAPACITY {
        for ctx in [&alpha, &beta] {
            land_compute_listing(&mut state, ctx, count);
            for observation in observe_services(&state) {
                history.record(observation, Instant::now());
            }
        }
    }

    for ctx in [&alpha, &beta] {
        assert_eq!(
            history
                .series_in(ResourceServiceKind::ComputeInstances, ctx)
                .len(),
            HISTORY_CAPACITY,
            "a context's series was cut short by another context's samples"
        );
    }
}

/// ...and a context the operator comes back to still has its history, up to the
/// number of contexts the history retains.
#[test]
fn a_context_returned_to_still_has_its_series() {
    let alpha = context("alpha");
    let beta = context("beta");
    let mut state = ResourcesState::default();
    let mut history = ServiceHealthHistory::default();

    for count in [1, 2] {
        land_compute_listing(&mut state, &alpha, count);
        for observation in observe_services(&state) {
            history.record(observation, Instant::now());
        }
    }
    for count in 0..HISTORY_CAPACITY + 5 {
        land_compute_listing(&mut state, &beta, count);
        for observation in observe_services(&state) {
            history.record(observation, Instant::now());
        }
    }

    assert_eq!(
        history.series_in(ResourceServiceKind::ComputeInstances, &alpha),
        vec![1, 2],
        "alpha's history was evicted by beta's polling"
    );
}

/// The retention bound is real, and it is on whole contexts: a session that
/// visits more of them than the history keeps loses the least recently sampled
/// one entirely rather than growing without limit.
#[test]
fn the_least_recently_sampled_context_is_dropped_once_the_bound_is_reached() {
    let contexts: Vec<_> = (0..HISTORY_CONTEXTS + 1)
        .map(|index| context(&format!("ctx-{index}")))
        .collect();
    let mut state = ResourcesState::default();
    let mut history = ServiceHealthHistory::default();

    for ctx in &contexts {
        for count in [1, 2] {
            land_compute_listing(&mut state, ctx, count);
            for observation in observe_services(&state) {
                history.record(observation, Instant::now());
            }
        }
    }

    assert_eq!(
        history.series_in(ResourceServiceKind::ComputeInstances, &contexts[0]),
        Vec::<usize>::new(),
        "the oldest context must have been dropped whole"
    );
    for ctx in &contexts[1..] {
        assert_eq!(
            history.series_in(ResourceServiceKind::ComputeInstances, ctx),
            vec![1, 2],
            "a retained context lost its series"
        );
    }
}

/// Derived from the registry, not from a hand-written service list: three
/// services once shipped with no Home entry and nothing failed. Every
/// kind in `ALL` must have a readable series, empty or not — a kind the history
/// does not know about would report "no history" forever.
#[test]
fn every_registered_service_has_a_readable_series() {
    let ctx = context("alpha");
    let history = ServiceHealthHistory::default();

    for kind in ResourceServiceKind::ALL {
        assert_eq!(
            history.series_in(kind, &ctx),
            Vec::<usize>::new(),
            "{kind:?} has no series to read"
        );
    }
}

/// Sample identity is the *landed result*, for every registered service — and
/// a load that has merely started is not one.
///
/// Walked over `ALL` rather than asserted per service for the usual reason,
/// and pinned directly rather than through `observe_services` because a service
/// whose identity comes from the load rather than the result is silently
/// wrong in a way no count reveals: it re-presents the rows already on hand as
/// a fresh observation every time a fetch is dispatched and dropped. Cloud
/// Storage reaches the counter through its own passthrough, so it is precisely
/// the arm a per-service test would not have covered.
#[test]
fn no_registered_service_reports_a_result_it_has_not_landed() {
    let mut state = ResourcesState::default();

    for kind in ResourceServiceKind::ALL {
        assert_eq!(
            state.list_landed_sequence(kind),
            None,
            "{kind:?} claims a result with nothing ever loaded"
        );
    }

    for kind in ResourceServiceKind::ALL {
        state.begin_load(kind, Instant::now());
        assert_eq!(
            state.list_landed_sequence(kind),
            None,
            "{kind:?} reported a started load as a landed result"
        );
    }
}

/// Cloud Storage lands its listing through its own state rather than through
/// the generic collection, and so reaches the landed-result identity through
/// its own passthrough.
///
/// Every other service in the registry shares one code path; this one is the
/// arm a compute-only fixture leaves unexercised, which is exactly where a
/// passthrough that answered "nothing has landed" forever would hide — the
/// service would sample once, never again, and its sparkline would sit at one
/// point looking like a service nobody had listed twice.
#[test]
fn a_landed_bucket_listing_is_observed_like_any_other() {
    let ctx = context("alpha");
    let mut state = ResourcesState::default();
    state.begin_load(ResourceServiceKind::CloudStorage, Instant::now());
    state
        .storage_mut()
        .set_buckets(ctx.clone(), vec![bucket("a"), bucket("b")]);

    let storage: Vec<_> = observe_services(&state)
        .into_iter()
        .filter(|observation| observation.kind == ResourceServiceKind::CloudStorage)
        .collect();

    assert_eq!(
        storage.len(),
        1,
        "the landed bucket listing is one observation"
    );
    assert_eq!(storage[0].total, 2);
    assert_eq!(storage[0].context, ctx);
}

/// ...and a landed result does advance it, with a fresh identity per landing so
/// two listings of the same size are two samples.
#[test]
fn each_landed_listing_reports_a_fresh_identity() {
    let ctx = context("alpha");
    let mut state = ResourcesState::default();

    land_compute_listing(&mut state, &ctx, 3);
    let first = state
        .list_landed_sequence(ResourceServiceKind::ComputeInstances)
        .expect("the landed listing has an identity");

    land_compute_listing(&mut state, &ctx, 3);
    let second = state
        .list_landed_sequence(ResourceServiceKind::ComputeInstances)
        .expect("the second landed listing has an identity");

    assert!(
        second > first,
        "two landings must not share an identity: got {first} then {second}"
    );
}

/// The clock is a parameter, the way `src/app/state/tunnels.rs` takes one, so
/// the sample's age is assertable without sleeping. Slice 3's decay window
/// reads these stamps.
#[test]
fn a_sample_carries_the_instant_it_was_recorded_at() {
    let ctx = context("alpha");
    let start = Instant::now();
    let mut state = ResourcesState::default();
    let mut history = ServiceHealthHistory::default();

    land_compute_listing(&mut state, &ctx, 1);
    for observation in observe_services(&state) {
        history.record(observation, start);
    }
    land_compute_listing(&mut state, &ctx, 2);
    for observation in observe_services(&state) {
        history.record(observation, start + Duration::from_secs(30));
    }

    let stamps: Vec<Duration> = history
        .samples_in(ResourceServiceKind::ComputeInstances, &ctx)
        .map(|sample| sample.at.duration_since(start))
        .collect();
    assert_eq!(
        stamps,
        vec![Duration::ZERO, Duration::from_secs(30)],
        "the recorded stamp must be the caller's instant, not a clock read inside the state"
    );
}
