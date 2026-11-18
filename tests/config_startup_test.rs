use std::io::Write;

use clap::Parser;
use nuvem::app::AppState;
use nuvem::app::projection::binding::help_model_with;
use nuvem::app::service_registry::ServiceId;
use nuvem::app::state::configurations::{
    ConfigurationContextPickerItem, ConfigurationContextPickerKind,
};
use nuvem::config::hotkeys::KeyOverrides;
use nuvem::services::runtime::{apply_startup_config, surface_config_problems};
use nuvem::{Cli, Config};
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use nuvem_domain::gcp::location::ComputeRegion;
use nuvem_domain::gcp::resource::ResourceServiceKind;

fn region_item(name: &str) -> ConfigurationContextPickerItem {
    ConfigurationContextPickerItem::Region(ComputeRegion {
        name: name.to_string(),
        status: None,
    })
}

/// An active configuration with a region set, so `start_context_edit` has
/// something to edit and the region picker can be opened against it.
fn config_with_active_region(region: &str) -> GcloudConfiguration {
    GcloudConfiguration {
        name: GcloudConfigurationName::new("dev"),
        is_active: true,
        account: Some("dev@example.com".to_string()),
        project: None,
        region: Some(region.to_string()),
        zone: None,
    }
}

/// A collector holding exactly the given messages, all from the same source:
/// the surfacing code only reads messages and their count.
fn problems_of(messages: &[&str]) -> Vec<String> {
    messages
        .iter()
        .map(|message| (*message).to_string())
        .collect()
}

fn write_config(body: &str) -> tempfile::NamedTempFile {
    let mut f = tempfile::NamedTempFile::new().expect("create temp config");
    f.write_all(body.as_bytes()).expect("write temp config");
    f.flush().expect("flush temp config");
    f
}

fn cli_for(path: &std::path::Path) -> Cli {
    Cli::parse_from([
        "nuvem",
        "--plugins",
        "/nonexistent/nuvem-plugins-test.toml",
        "--config",
        path.to_str().unwrap(),
    ])
}

/// `default_service = "gke"` starts the resources workspace on GKE.
#[test]
fn default_service_from_config_starts_active_service() {
    let file = write_config("version = 1\n[general]\ndefault_service = \"gke\"\n");
    let cli = cli_for(file.path());
    let config = Config::resolve(&cli).config;
    assert_eq!(config.default_service, ResourceServiceKind::GkeClusters);

    let mut app = AppState::new();
    apply_startup_config(&mut app, &config);

    assert_eq!(
        app.root.resources.active_service(),
        ResourceServiceKind::GkeClusters
    );
}

/// The default config (no override) still starts on GCE.
#[test]
fn default_service_defaults_to_gce_with_no_config_file() {
    let cli = cli_for(std::path::Path::new("/nonexistent/nuvem-startup-test.toml"));
    let config = Config::resolve(&cli).config;

    let mut app = AppState::new();
    apply_startup_config(&mut app, &config);

    assert_eq!(
        app.root.resources.active_service(),
        ResourceServiceKind::ComputeInstances
    );
}

/// `sidebar_visible = false` starts the service navigator with the sidebar
/// hidden.
#[test]
fn sidebar_visible_false_from_config_hides_sidebar_at_startup() {
    let file = write_config("version = 1\n[general]\nsidebar_visible = false\n");
    let cli = cli_for(file.path());
    let config = Config::resolve(&cli).config;
    assert!(!config.sidebar_visible);

    let mut app = AppState::new();
    apply_startup_config(&mut app, &config);

    assert!(!app.root.navigator.sidebar_visible());
}

/// With no config file, the sidebar remains visible by default.
#[test]
fn sidebar_visible_defaults_to_true_with_no_config_file() {
    let cli = cli_for(std::path::Path::new("/nonexistent/nuvem-startup-test.toml"));
    let config = Config::resolve(&cli).config;

    let mut app = AppState::new();
    apply_startup_config(&mut app, &config);

    assert!(app.root.navigator.sidebar_visible());
}

/// Aliases from the config file are wired into `root.aliases` at startup.
#[test]
fn aliases_from_config_are_applied_at_startup() {
    let file = write_config("version = 1\n[aliases]\nvms = \"gce\"\n");
    let cli = cli_for(file.path());
    let config = Config::resolve(&cli).config;

    let mut app = AppState::new();
    apply_startup_config(&mut app, &config);

    assert_eq!(
        app.root.aliases.get("vms"),
        Some(ServiceId::ComputeInstances)
    );
}

/// The `J` picker derives its rows from `RootState::jump_rules`, so a
/// `[[jumps]]` array that never reaches the app state would leave the user's
/// file with no effect whatsoever.
#[test]
fn jump_rules_from_config_are_applied_at_startup() {
    let file = write_config("version = 1\n[[jumps]]\nfrom = \"gke\"\nto = \"gke:node-pools\"\n");
    let cli = cli_for(file.path());
    let config = Config::resolve(&cli).config;

    let mut app = AppState::new();
    apply_startup_config(&mut app, &config);

    assert_eq!(app.root.jump_rules, config.jumps);
    assert_eq!(
        app.root
            .jump_rules
            .rules()
            .iter()
            .map(|rule| rule.to.config_key())
            .collect::<Vec<_>>(),
        vec!["gke:node-pools".to_string()]
    );
}

/// Favorites from the config file's `[favorites]` table are wired into
/// `RootState::configurations` at startup: the region picker, opened right
/// after, must already see them, or every picker would open with an empty
/// favorites list until the next reload.
#[test]
fn favorites_from_config_are_installed_at_startup_and_used_by_the_picker() {
    let file = write_config("version = 1\n[favorites]\nregions = [\"europe-west1\"]\n");
    let cli = cli_for(file.path());
    let config = Config::resolve(&cli).config;

    let mut app = AppState::new();
    app.root
        .configurations
        .set_rows(vec![config_with_active_region("europe-west1")]);
    assert!(app.root.configurations.start_context_edit());
    // Project -> Region.
    app.root.configurations.focus_next_context_edit_field();

    apply_startup_config(&mut app, &config);

    let kind = app
        .root
        .configurations
        .open_context_picker()
        .expect("context edit form is open");
    assert_eq!(kind, ConfigurationContextPickerKind::Region);
    let picker = app
        .root
        .configurations
        .context_picker()
        .expect("picker is open");
    assert!(picker.is_favorite(&region_item("europe-west1")));
    assert!(!picker.is_favorite(&region_item("us-central1")));
}

/// A load-level config error (unparseable file / unsupported version) is
/// surfaced as an error status message, so a bad config is visible without
/// crashing the app.
#[test]
fn load_error_is_surfaced_as_error_status_message() {
    let mut app = AppState::new();
    surface_config_problems(
        &mut app,
        &problems_of(&["config file unreadable"]),
        Some("boom: unreadable config"),
        &KeyOverrides::empty(),
    );

    let status = app
        .root
        .system
        .status_message
        .clone()
        .expect("status message set");
    assert_eq!(status, "config error: boom: unreadable config");
    // The error wins the status line, but the details still reach the help
    // overlay rather than being dropped.
    assert_eq!(
        app.root.system.config_problems(),
        ["config file unreadable"]
    );
}

/// A field-level warning (bad enum/alias/hotkey, with the field defaulted)
/// is surfaced as a plain status message, distinct from a hard load error.
#[test]
fn field_level_warning_is_surfaced_as_status_message() {
    let mut app = AppState::new();
    surface_config_problems(
        &mut app,
        &problems_of(&["unknown general.theme \"bogus\"; using default"]),
        None,
        &KeyOverrides::empty(),
    );

    let status = app
        .root
        .system
        .status_message
        .clone()
        .expect("status message set");
    // Byte-identical to the earlier single-warning wording.
    assert_eq!(
        status,
        "config warning: unknown general.theme \"bogus\"; using default"
    );
}

/// More than one problem: the status line stops pretending there is only one
/// and points at the help overlay, which lists every message.
#[test]
fn multiple_problems_are_counted_in_the_status_and_all_listed_in_help() {
    let mut app = AppState::new();
    surface_config_problems(
        &mut app,
        &problems_of(&[
            "unknown general.default_service \"nope\"; using default",
            "unknown general.theme \"bogus\"; using default",
        ]),
        None,
        &KeyOverrides::empty(),
    );

    assert_eq!(
        app.root.system.status_message.as_deref(),
        Some("config: 2 problems (press ? for details)")
    );
    assert_eq!(
        help_model_with(&app.root, &KeyOverrides::empty()).problems,
        [
            "unknown general.default_service \"nope\"; using default",
            "unknown general.theme \"bogus\"; using default",
        ]
    );
}

/// The pointer names the *running* help key, so a rebound help key is what the
/// user is told to press.
#[test]
fn the_multi_problem_pointer_names_the_rebound_help_key() {
    let file = write_config("version = 1\n[hotkeys]\nhelp = \"H\"\n");
    let cli = cli_for(file.path());
    let outcome = Config::resolve(&cli);
    assert!(
        outcome.problems.is_empty(),
        "the hotkey override must itself validate: {:?}",
        outcome.problems.messages()
    );

    let mut app = AppState::new();
    surface_config_problems(
        &mut app,
        &problems_of(&["first problem", "second problem"]),
        None,
        &outcome.config.hotkeys,
    );

    let status = app
        .root
        .system
        .status_message
        .clone()
        .expect("status message set");
    assert_eq!(status, "config: 2 problems (press H for details)");
    assert!(
        !status.contains('?'),
        "the default key must not be named once help is rebound: {status:?}"
    );
}

/// A clean load replaces any previously recorded problems, so the help
/// overlay can never keep showing problems the current config no longer has.
#[test]
fn an_empty_problem_list_clears_previously_recorded_problems() {
    let mut app = AppState::new();
    app.root
        .system
        .set_config_problems(vec!["stale problem".to_string()]);

    surface_config_problems(&mut app, &[], None, &KeyOverrides::empty());

    assert!(app.root.system.config_problems().is_empty());
    assert!(
        help_model_with(&app.root, &KeyOverrides::empty())
            .problems
            .is_empty()
    );
}

/// A service contributes one message however many keys named it, so that
/// message has to describe what actually happened to the table — here it is
/// also the single problem, and so the whole status line. Two config keys
/// name compute and the deciding one (`gce`, alphabetically last) is broken: the service falls back
/// to its built-in columns whole, and the one message the user gets says why
/// rather than claiming `columns.gce` decides a table it never configured.
#[test]
fn a_rejected_deciding_block_surfaces_its_rejection_not_the_duplicate_key() {
    let file = write_config(
        "version = 1\n\n\
         [columns.compute]\ncolumns = [\"name\", \"status\"]\n\n\
         [columns.gce]\ncolumns = [\"name\", \"zzz\"]\n",
    );
    let cli = cli_for(file.path());
    let outcome = Config::resolve(&cli);

    let mut app = AppState::new();
    apply_startup_config(&mut app, &outcome.config);
    surface_config_problems(
        &mut app,
        &outcome.problems.clone().into_messages(),
        outcome.load_error.as_deref(),
        &outcome.config.hotkeys,
    );

    let status = app
        .root
        .system
        .status_message
        .clone()
        .expect("status message set");
    assert!(
        status.contains("columns.gce: unknown column \"zzz\""),
        "the surfaced message must name the rejection: {status:?}"
    );
    assert!(
        status.contains("ignoring this service's column settings"),
        "the surfaced message must say the service fell back: {status:?}"
    );
    assert!(
        !status.contains("columns.gce decides"),
        "the surfaced message must not claim the rejected block decides: {status:?}"
    );
    assert!(
        status.contains("columns.compute"),
        "the surfaced message must still name the block discarded with it: {status:?}"
    );

    // The message and the table agree: compute is on its built-in columns.
    assert_eq!(
        outcome
            .config
            .columns
            .get(ResourceServiceKind::ComputeInstances),
        None
    );
    assert!(
        outcome
            .config
            .columns
            .is_invalid(ResourceServiceKind::ComputeInstances)
    );
}

/// With no warnings and no load error, startup leaves the status message
/// untouched.
#[test]
fn no_problems_leaves_status_message_untouched() {
    let mut app = AppState::new();
    surface_config_problems(&mut app, &[], None, &KeyOverrides::empty());

    assert!(app.root.system.status_message.is_none());
}
