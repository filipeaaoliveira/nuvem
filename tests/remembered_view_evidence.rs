//! End-to-end evidence: the remembered view survives a process
//! boundary, which is the one claim no unit test in the reducer can make.
//!
//! Temp directories come from `tempfile`, the idiom
//! `tests/state_writer_evidence.rs` already settled on: a fixed shared
//! path collides when two `cargo test` runs overlap.

use nuvem::Config;
use nuvem::app::reducer::root::reduce;
use nuvem::app::state::remembered_view::RememberedViews;
use nuvem::app::{Action, AppState, Effect, EffectResult, ResourcesAction};
use nuvem::config::problems::ConfigProblems;
use nuvem::config::state;
use nuvem::services::runtime::{apply_startup_config, load_remembered_views};
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use nuvem_domain::gcp::resource::ResourceServiceKind;

#[test]
fn a_two_configuration_map_round_trips_through_the_real_state_writer() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("state.toml");

    let mut views = RememberedViews::default();
    views.set(
        GcloudConfigurationName::new("dev"),
        ResourceServiceKind::CloudSql,
    );
    views.set(
        GcloudConfigurationName::new("prod"),
        ResourceServiceKind::GkeClusters,
    );

    state::save(&path, &views).expect("save");
    let reloaded: Option<RememberedViews> = state::load(&path).expect("load");

    assert_eq!(reloaded, Some(views));
}

#[test]
fn the_state_file_names_services_the_way_the_config_file_does() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("state.toml");

    let mut views = RememberedViews::default();
    views.set(
        GcloudConfigurationName::new("dev"),
        ResourceServiceKind::CloudSql,
    );
    state::save(&path, &views).expect("save");

    let written = std::fs::read_to_string(&path).expect("read back");
    assert!(
        written.contains("dev = \"cloudsql\""),
        "the state file must use the config vocabulary, got:\n{written}"
    );
    assert!(
        !written.contains("CloudSql"),
        "serde's derived variant name must not reach disk, got:\n{written}"
    );
}

#[test]
fn a_missing_state_file_is_not_an_error() {
    let dir = tempfile::tempdir().expect("temp dir");

    let loaded: Option<RememberedViews> =
        state::load(&dir.path().join("state.toml")).expect("a missing file must not be an error");

    assert_eq!(loaded, None);
}

/// The process boundary itself: a state file left by a previous run decides
/// which service this run opens on, beating `general.default_service`. The
/// startup load and the reducer's choke point are both real here — only the
/// terminal and `initialize()`'s gcloud call are stood in for, by feeding the
/// `ConfigurationsLoaded` result they would have produced.
#[test]
fn a_state_file_written_before_a_restart_decides_the_view_the_app_opens_on() {
    let dir = tempfile::tempdir().expect("temp dir");
    let config_path = dir.path().join("config.toml");

    // The previous run's memory, written by the app's own writer.
    let mut views = RememberedViews::default();
    views.set(
        GcloudConfigurationName::new("prod"),
        ResourceServiceKind::GkeClusters,
    );
    state::save(&state::state_path_for(&config_path), &views).expect("save the previous run");

    // This run: the resolved config, the startup application of it, and the
    // startup load — the three things `run()` does before `initialize()`.
    let config = Config {
        config_path,
        default_service: ResourceServiceKind::ComputeInstances,
        ..Config::default()
    };
    let mut problems = ConfigProblems::default();
    let mut app = AppState::new();
    apply_startup_config(&mut app, &config);
    app.root.remembered_views = load_remembered_views(&config, &mut problems).views;

    // Then `initialize()`'s first configuration landing.
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![GcloudConfiguration {
                name: GcloudConfigurationName::new("prod"),
                is_active: true,
                account: None,
                project: Some("prod-project".to_string()),
                region: None,
                zone: None,
            }],
        }),
    );

    assert_eq!(
        app.root.resources.active_service(),
        ResourceServiceKind::GkeClusters,
        "the app must open on the service the previous run left in state.toml, \
         not on general.default_service"
    );
    assert!(
        problems.iter().next().is_none(),
        "a readable state file must not raise a config problem, got {:?}",
        problems.messages()
    );
}

/// A state file this build cannot read must survive the session untouched.
///
/// `src/config/state.rs`'s decision 4 exists so a state file written by a newer
/// nuvem does not break an older one — but ignoring it on read is only half of
/// that promise. If the session then saves, the first service switch replaces
/// the newer build's file with a `version = 1` one, and the guard has bought
/// nothing. The session still behaves normally in memory; only the write is
/// suppressed.
#[test]
fn a_state_file_this_build_cannot_read_is_left_intact() {
    let dir = tempfile::tempdir().expect("temp dir");
    let config_path = dir.path().join("config.toml");
    let state_path = state::state_path_for(&config_path);
    // Deliberately parseable as a `RememberedViews` payload: the version, not
    // the payload shape, is what has to stop this being adopted or replaced.
    let written_by_a_newer_nuvem = "version = 2\n[data]\nprod = \"gke\"\n";
    std::fs::write(&state_path, written_by_a_newer_nuvem).expect("write the newer run's state");

    let config = Config {
        config_path,
        default_service: ResourceServiceKind::ComputeInstances,
        ..Config::default()
    };
    let mut problems = ConfigProblems::default();
    let mut app = AppState::new();
    apply_startup_config(&mut app, &config);
    let startup = load_remembered_views(&config, &mut problems);
    app.root.remembered_views = startup.views;
    app.root.remembered_view_saves_suppressed = startup.saves_suppressed;

    // The suppression below is invisible to the user unless the warning names
    // it: without this the message reads as a one-off degraded read, while in
    // fact the feature is dead for this session and every later one until the
    // file is removed. The save-failure path already sets that precedent.
    let warning = problems
        .messages()
        .into_iter()
        .find(|message| message.contains("state.toml"))
        .expect("an unreadable state file must be reported to the user");
    assert!(
        warning.contains("will not save the remembered view this session"),
        "the warning must say saving is off for the session, got {warning:?}"
    );
    assert!(
        warning.contains("in case a newer nuvem wrote that file"),
        "the warning must name why saving is off, got {warning:?}"
    );
    // Deliberately not advised: for the case this suppression exists to
    // protect, deleting the file is exactly the data loss it prevents.
    assert!(
        !warning.contains("remove") && !warning.contains("delete"),
        "the warning must not advise destroying the file it is protecting, got {warning:?}"
    );
    assert!(
        !warning.contains('\n'),
        "the warning shares a one-line status surface, got {warning:?}"
    );

    // `initialize()`'s first configuration landing, then a service switch —
    // the keypress that would overwrite the file.
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![GcloudConfiguration {
                name: GcloudConfigurationName::new("prod"),
                is_active: true,
                account: None,
                project: Some("prod-project".to_string()),
                region: None,
                zone: None,
            }],
        }),
    );
    let effects = reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::CloudSql,
        )),
    );

    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::SaveRememberedViews { .. })),
        "a state file this build could not read must not be written over, got {effects:?}"
    );
    assert_eq!(
        std::fs::read_to_string(&state_path).expect("read back"),
        written_by_a_newer_nuvem,
        "the newer build's state file must be byte-for-byte untouched"
    );
    // The session is only denied the *write*: the view still follows the user.
    assert_eq!(
        app.root.resources.active_service(),
        ResourceServiceKind::CloudSql,
        "suppressing the save must not suppress the in-session view"
    );
    assert_eq!(
        app.root
            .remembered_views
            .get(&GcloudConfigurationName::new("prod")),
        Some(ResourceServiceKind::CloudSql),
        "the in-memory map must still learn, so the session behaves normally"
    );
}

/// The startup warning for an unreadable state file, rendered end to end.
///
/// `src/config/state.rs` composes the first half and the runtime appends the
/// second, so neither half alone shows what the user reads. The existing
/// coverage asserts substrings, which is blind to the two halves saying the
/// same thing twice — and they did: the version-mismatch message already ends
/// with "starting from defaults", which the appended half then repeated.
///
/// Both load-failure shapes are pinned here because `state.rs` composes a
/// different first half for a corrupt file than for a future version, and a
/// tail that reads well after one can read badly after the other.
#[test]
fn the_unreadable_state_warning_reads_as_one_sentence() {
    let future_version = startup_warning_for("version = 2\n[data]\nprod = \"gke\"\n");
    let corrupt = startup_warning_for("this is not toml at all\n");

    assert!(
        future_version.ends_with(
            "state version 2 was written by a newer nuvem (this build reads 1); ignoring it \
             and starting from defaults; nuvem will not save the remembered view this session, \
             in case a newer nuvem wrote that file"
        ),
        "the two halves must join into one sentence, got {future_version:?}"
    );
    for warning in [&future_version, &corrupt] {
        assert_eq!(
            warning.matches("starting from defaults").count(),
            usize::from(warning.contains("starting from defaults")),
            "the runtime must not restate what the state reader already said, got {warning:?}"
        );
        assert!(
            warning.ends_with(
                "nuvem will not save the remembered view this session, in case a newer nuvem \
                 wrote that file"
            ),
            "every load failure must still say saving is off, and why, got {warning:?}"
        );
        assert!(
            !warning.contains('\n'),
            "the warning shares a one-line status surface, got {warning:?}"
        );
    }
}

/// The warning a startup load of `contents` puts in front of the user, or a
/// panic if that load unexpectedly succeeded.
fn startup_warning_for(contents: &str) -> String {
    let dir = tempfile::tempdir().expect("temp dir");
    let config_path = dir.path().join("config.toml");
    std::fs::write(state::state_path_for(&config_path), contents).expect("write the state file");

    let config = Config {
        config_path,
        ..Config::default()
    };
    let mut problems = ConfigProblems::default();
    let startup = load_remembered_views(&config, &mut problems);
    assert!(
        startup.saves_suppressed,
        "this helper is about the load-failure path"
    );
    problems
        .messages()
        .into_iter()
        .find(|message| message.contains("state.toml"))
        .expect("an unreadable state file must be reported to the user")
        .to_string()
}

/// The other half of the load rule, and the path almost every run takes: no
/// state file yet. "Absent" and "there but unreadable" both produce an empty
/// map, and the suppression above is the only thing that separates them — so
/// suppressing on absence too would silently disable the feature for every user
/// who has never had a state file, on their first run and on every run after
/// it, because nothing would ever write the file that would end the drought.
///
/// This is asserted here rather than left to the other tests in this file: they
/// all exercise a state file that exists, so none of them would notice.
#[test]
fn a_first_run_with_no_state_file_still_saves() {
    let dir = tempfile::tempdir().expect("temp dir");
    let config_path = dir.path().join("config.toml");
    let state_path = state::state_path_for(&config_path);
    assert!(
        !state_path.exists(),
        "this test is about the absent-file case; the fixture must not have one"
    );

    let config = Config {
        config_path,
        default_service: ResourceServiceKind::ComputeInstances,
        ..Config::default()
    };
    let mut problems = ConfigProblems::default();
    let mut app = AppState::new();
    apply_startup_config(&mut app, &config);
    let startup = load_remembered_views(&config, &mut problems);
    app.root.remembered_views = startup.views;
    app.root.remembered_view_saves_suppressed = startup.saves_suppressed;

    assert!(
        !startup.saves_suppressed,
        "a missing file is a first run, not a file we must protect from being clobbered"
    );
    assert!(
        problems.iter().next().is_none(),
        "a first run has no state yet; that is not a problem to report, got {:?}",
        problems.messages()
    );

    // The flag only matters through what it stops, so drive the same landing
    // and keypress the suppressed case does and assert the save is emitted.
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![GcloudConfiguration {
                name: GcloudConfigurationName::new("prod"),
                is_active: true,
                account: None,
                project: Some("prod-project".to_string()),
                region: None,
                zone: None,
            }],
        }),
    );
    let effects = reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::CloudSql,
        )),
    );

    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::SaveRememberedViews { .. })),
        "a first run must write the state file it did not find, got {effects:?}"
    );
}
