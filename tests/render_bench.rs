//! Times the render path the run loop actually executes, so a change to it can
//! be measured rather than argued about.
//!
//! Run by hand — it is `#[ignore]`d, asserts nothing, and only prints:
//!
//! ```text
//! cargo nextest run --release -E 'test(render_bench)' --run-ignored all --no-capture
//! ```
//!
//! It lives under `tests/` rather than in `examples/` or `benches/` so that it
//! is mounted in the one suite binary and built, linted and formatted with the
//! rest of the tests, instead of as a target nothing else exercises.
//!
//! Timings are reported as the MINIMUM across batches, not the mean. This is a
//! latency question and the machine only ever adds to it, so the fastest batch
//! is the one least polluted by whatever else was running. Means moved 25% run
//! to run on the machine this was written on, which is enough to report a
//! change as an improvement when it is noise.
use nuvem::app::reducer::root::reduce;
use nuvem::app::{Action, AppState, EffectResult, ResourcesAction, WorkspaceAction};
use nuvem::services::runtime::session::RuntimeSession;
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use nuvem_domain::gcp::resource::{ComputeInstance, ResourceServiceKind, ResourceStatus};
use ratatui::{Terminal, backend::TestBackend};
use std::time::Instant;

fn app_with(rows: usize) -> AppState {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![GcloudConfiguration {
                name: GcloudConfigurationName::new("dev"),
                is_active: true,
                account: Some("dev@example.com".into()),
                project: Some("dev-project".into()),
                region: None,
                zone: None,
            }],
        }),
    );
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Resources));
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::ComputeInstances,
        )),
    );
    let context = app.root.current_context().expect("context");
    let instances = (0..rows)
        .map(|i| ComputeInstance {
            id: format!("{i}"),
            name: format!("api-prod-instance-{i:04}"),
            zone: "europe-west1-b".into(),
            status: ResourceStatus::Running,
            machine_type: "e2-standard-4".into(),
            internal_ip: Some(format!("10.0.{}.{}", i / 256, i % 256)),
            external_ip: Some(format!("34.10.{}.{}", i / 256, i % 256)),
        })
        .collect();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded { context, instances }),
    );
    app
}

fn bench(label: &str, rows: usize, iters: u32) {
    let session = RuntimeSession::new_for_test(app_with(rows));
    let mut terminal = Terminal::new(TestBackend::new(200, 50)).expect("terminal");
    // Warm up so the first allocation burst is not in the sample.
    for _ in 0..50 {
        terminal.draw(|f| nuvem::ui::render(f, &session)).unwrap();
    }
    // The MINIMUM across batches, not the mean: this is a latency question and
    // the machine only ever adds time to it, so the fastest batch is the one
    // least polluted by whatever else the laptop was doing. Means here move by
    // 25% run to run and would have reported an improvement that was noise.
    let mut each = std::time::Duration::MAX;
    for _ in 0..9 {
        let start = Instant::now();
        for _ in 0..iters {
            terminal.draw(|f| nuvem::ui::render(f, &session)).unwrap();
        }
        each = each.min(start.elapsed() / iters);
    }
    println!(
        "{label:28} {rows:>4} rows  {:>9.3?}/frame   {:>6.1} frames/s",
        each,
        1.0 / each.as_secs_f64()
    );
}

fn bench_projection(rows: usize, iters: u32) {
    let app = app_with(rows);
    let mut sink = 0usize;
    let mut each = std::time::Duration::MAX;
    for _ in 0..9 {
        let start = Instant::now();
        for _ in 0..iters {
            let p = nuvem::app::projection::project_resources(&app.root.resources);
            sink += p.visible_rows;
        }
        each = each.min(start.elapsed() / iters);
    }
    println!("  project_resources only     {rows:>4} rows  {each:>9.3?}  (sink {sink})");
}

fn bench_parts(rows: usize, iters: u32) {
    let app = app_with(rows);
    let overrides = Default::default();
    let mut t = std::time::Duration::MAX;
    for _ in 0..9 {
        let start = Instant::now();
        for _ in 0..iters {
            let p = nuvem::app::projection::project_app_frame_with(&app.root, &overrides);
            std::hint::black_box(&p);
        }
        t = t.min(start.elapsed() / iters);
    }
    println!("  project_app_frame_with     {rows:>4} rows  {t:>9.3?}");
}

#[test]
#[ignore = "a measurement, not an assertion; run it by hand"]
fn render_cost_by_row_count() {
    bench("resources list", 0, 500);
    bench("resources list", 50, 500);
    bench("resources list", 500, 200);
    bench_projection(0, 500);
    bench_projection(50, 500);
    bench_projection(500, 200);
    bench_parts(0, 2000);
    bench_parts(500, 2000);
}
