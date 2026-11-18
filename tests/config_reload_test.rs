use std::io::Write;

use clap::Parser;
use nuvem::app::state::configurations::{
    ConfigurationContextPickerItem, ConfigurationContextPickerKind,
};
use nuvem::app::{Action, AppState, Effect};
use nuvem::config::favorites::FavoriteKind;
use nuvem::services::runtime::session::RuntimeSession;
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

fn write_config(body: &str) -> tempfile::NamedTempFile {
    let mut f = tempfile::NamedTempFile::new().expect("create temp config");
    f.write_all(body.as_bytes()).expect("write temp config");
    f.flush().expect("flush temp config");
    f
}

fn cli_for(path: &std::path::Path) -> Cli {
    Cli::parse_from(["nuvem", "--config", path.to_str().unwrap()])
}

// The live theme swap is asserted in `config_reload_theme_test.rs`: the theme
// is a process-global arc-swap, so its assertions need a process no other
// reload (which also calls `set_active`) shares — under the consolidated
// suite target that isolation comes from nextest's process-per-test runner,
// not from a dedicated binary.

/// A restart-required field (`default_service`) is reported but not applied
/// live, and the running config keeps its previous value.
#[tokio::test]
async fn reload_reports_restart_required_without_applying() {
    let file = write_config("version = 1\n[general]\ndefault_service = \"gce\"\n");
    let cli = cli_for(file.path());
    let config = Config::resolve(&cli).config;
    assert_eq!(
        config.default_service,
        ResourceServiceKind::ComputeInstances
    );

    let mut session =
        RuntimeSession::new_with_cli_and_config_for_test(cli, config, AppState::new());

    std::fs::write(
        file.path(),
        "version = 1\n[general]\ndefault_service = \"gke\"\n",
    )
    .expect("rewrite temp config");

    session
        .dispatch_action(Action::Effect(Effect::ReloadConfig))
        .await;

    // Restart-required field is NOT applied live.
    assert_eq!(
        session.config().unwrap().default_service,
        ResourceServiceKind::ComputeInstances
    );

    let status = session
        .app()
        .root
        .system
        .status_message
        .clone()
        .unwrap_or_default();
    assert!(
        status.contains("restart required") && status.contains("default_service"),
        "unexpected status message: {status:?}"
    );
}

/// Changing the poll interval is a safe field: it updates the stored config
/// and flags the run loop to rebuild its timer.
#[tokio::test]
async fn reload_updates_poll_interval_and_flags_run_loop() {
    let file = write_config("version = 1\n[general]\npoll_interval_secs = 5\n");
    let cli = cli_for(file.path());
    let config = Config::resolve(&cli).config;

    let mut session =
        RuntimeSession::new_with_cli_and_config_for_test(cli, config, AppState::new());

    std::fs::write(
        file.path(),
        "version = 1\n[general]\npoll_interval_secs = 30\n",
    )
    .expect("rewrite temp config");

    session
        .dispatch_action(Action::Effect(Effect::ReloadConfig))
        .await;

    assert_eq!(session.config().unwrap().poll_interval_secs, 30);
    assert!(session.take_poll_interval_dirty());
    // The flag is cleared once taken.
    assert!(!session.take_poll_interval_dirty());
}

/// Jump rules are a safe field: a rewritten `[[jumps]]` array takes effect in
/// the app state the picker reads *and* in the stored config, so the running
/// app and `nuvem info` cannot disagree about which jumps exist.
#[tokio::test]
async fn reload_adopts_new_jump_rules_live() {
    let file = write_config("version = 1\n[[jumps]]\nfrom = \"compute\"\nto = \"compute:disks\"\n");
    let cli = cli_for(file.path());
    let config = Config::resolve(&cli).config;

    let mut session =
        RuntimeSession::new_with_cli_and_config_for_test(cli, config, AppState::new());

    std::fs::write(
        file.path(),
        "version = 1\n[[jumps]]\nfrom = \"compute\"\nto = \"compute:metadata\"\n",
    )
    .expect("rewrite temp config");

    session
        .dispatch_action(Action::Effect(Effect::ReloadConfig))
        .await;

    let stored = session.config().unwrap().jumps.clone();
    assert_eq!(
        stored
            .rules()
            .iter()
            .map(|rule| rule.to.config_key())
            .collect::<Vec<_>>(),
        vec!["compute:metadata".to_string()]
    );
    assert_eq!(session.app().root.jump_rules, stored);
}

/// Favorites are a safe field, applied live exactly like aliases and jump
/// rules: a rewritten `[favorites]` table takes effect both in the running
/// picker (so the reorder is visible without a restart) and in the stored
/// config (so `nuvem info` cannot report favorites the picker has not
/// actually adopted).
#[tokio::test]
async fn reload_adopts_new_favorite_scopes_live() {
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
    app.root
        .configurations
        .set_favorites(config.favorites.clone());

    let mut session = RuntimeSession::new_with_cli_and_config_for_test(cli, config, app);

    std::fs::write(
        file.path(),
        "version = 1\n[favorites]\nregions = [\"us-central1\"]\n",
    )
    .expect("rewrite temp config");

    session
        .dispatch_action(Action::Effect(Effect::ReloadConfig))
        .await;

    // The stored config reflects the new favorites, so `nuvem info` cannot
    // disagree with what the picker just adopted.
    let stored = session.config().unwrap().favorites.clone();
    assert_eq!(
        stored.values(FavoriteKind::Region),
        ["us-central1".to_string()]
    );

    // The live picker reads the reloaded favorites, not the ones installed
    // at startup.
    let kind = session
        .app_mut()
        .root
        .configurations
        .open_context_picker()
        .expect("context edit form is open");
    assert_eq!(kind, ConfigurationContextPickerKind::Region);
    let picker = session
        .app()
        .root
        .configurations
        .context_picker()
        .expect("picker is open");
    assert!(picker.is_favorite(&region_item("us-central1")));
    assert!(!picker.is_favorite(&region_item("europe-west1")));
}

/// `gcloud_binary` and `log_file` are CLI-only fields (immutable across a
/// reload since the CLI args are re-parsed unchanged). The stored config must
/// keep the prior values rather than silently adopting whatever the fresh
/// resolve produced for them.
#[tokio::test]
async fn reload_does_not_adopt_gcloud_binary_or_log_file() {
    let file = write_config("version = 1\n[general]\ntheme = \"default\"\n");
    let cli = cli_for(file.path());
    let mut config = Config::resolve(&cli).config;
    // Simulate a config value that would not come from a fresh resolve of
    // this same CLI/file pair, so a reload that "adopts" it live would be
    // observably wrong.
    config.gcloud_binary = std::path::PathBuf::from("/custom/gcloud");
    config.log_file = std::path::PathBuf::from("/custom/nuvem.log");

    let mut session =
        RuntimeSession::new_with_cli_and_config_for_test(cli, config, AppState::new());

    std::fs::write(
        file.path(),
        "version = 1\n[general]\ntheme = \"high-contrast\"\n",
    )
    .expect("rewrite temp config");

    session
        .dispatch_action(Action::Effect(Effect::ReloadConfig))
        .await;

    let reloaded = session.config().unwrap();
    assert_eq!(
        reloaded.gcloud_binary,
        std::path::PathBuf::from("/custom/gcloud")
    );
    assert_eq!(
        reloaded.log_file,
        std::path::PathBuf::from("/custom/nuvem.log")
    );
}

/// A parse error in the reloaded file keeps the prior config and surfaces the
/// problem in the status line without crashing.
#[tokio::test]
async fn reload_with_bad_file_keeps_prior_config() {
    let file = write_config("version = 1\n[general]\npoll_interval_secs = 7\n");
    let cli = cli_for(file.path());
    let config = Config::resolve(&cli).config;
    assert_eq!(config.poll_interval_secs, 7);

    let mut session =
        RuntimeSession::new_with_cli_and_config_for_test(cli, config, AppState::new());

    std::fs::write(file.path(), "this is not valid toml = = =\n").expect("rewrite temp config");

    session
        .dispatch_action(Action::Effect(Effect::ReloadConfig))
        .await;

    // Prior config is preserved entirely (load-level failure bails outright).
    assert_eq!(session.config().unwrap().poll_interval_secs, 7);
    let status = session
        .app()
        .root
        .system
        .status_message
        .clone()
        .unwrap_or_default();
    assert!(
        status.contains("config reload failed"),
        "expected a load-failure status message, got: {status:?}"
    );
}
