//! CLI arguments and the layered resolution of runtime settings.
//!
//! Built-in defaults are folded over the optional TOML config file, which is
//! folded over CLI flags ([`ResolvedConfig::resolve`]); flags win. This module
//! also owns the `nuvem info` subcommand, the config file schema (themes,
//! hotkeys, aliases, favourite picker scopes, per-service columns and default
//! sort, jump rules), and the separate plugin file's schema.
//!
//! Persistence runs in both directions, under deliberately different rules:
//! `state.rs` writes state the app learns for itself to a `state.toml` beside
//! the config file in use, so a feature that needs memory across sessions
//! never rewrites the TOML the operator hand-edits, while `plugins.rs` is
//! *not* resolved relative to `--config`. Read those modules' own docs before
//! changing where either file is found.
//!
//! Resolution stops at "this declaration is well-formed": whether a
//! declaration applies to the row under the cursor is [`crate::app`]'s
//! question, not this module's.

pub mod aliases;
pub mod columns;
pub mod favorites;
pub mod file;
pub mod hotkeys;
pub mod jumps;
pub mod plugins;
pub mod problems;
pub mod read;
pub mod state;
pub mod theme;

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use nuvem_domain::gcp::resource::ResourceServiceKind;

use crate::app::service_registry::{self, ServiceId, ServiceTarget};

use aliases::AliasTable;
use columns::ColumnSettings;
use favorites::FavoriteScopes;
use hotkeys::KeyOverrides;
use jumps::JumpRules;
use plugins::Plugins;
use problems::{ConfigProblemSource, ConfigProblems};
use theme::{ThemeChoice, ThemeOverrides};

/// Command-line interface. Flags take precedence over the config file, which
/// in turn takes precedence over built-in defaults (see [`ResolvedConfig::resolve`]).
#[derive(Parser, Debug)]
#[command(name = "nuvem", version, about = "Terminal UI for Google Cloud")]
pub struct Cli {
    #[arg(long)]
    pub configuration: Option<String>,

    #[arg(
        long,
        help = "Seconds between background refreshes of the active service; 0 disables background refresh"
    )]
    pub poll_interval: Option<u64>,

    #[arg(long)]
    pub log_file: Option<PathBuf>,

    #[arg(long)]
    pub gcloud_binary: Option<PathBuf>,

    #[arg(long, help = "Path to the bq binary used for BigQuery browsing")]
    pub bq_binary: Option<PathBuf>,

    #[arg(long)]
    pub k9s_binary: Option<PathBuf>,

    #[arg(long)]
    pub kubectl_binary: Option<PathBuf>,

    #[arg(
        long,
        help = "Path to the Cloud SQL Auth Proxy binary used by managed SQL tunnels"
    )]
    pub cloud_sql_proxy_binary: Option<PathBuf>,

    #[arg(
        long,
        help = "Maximum number of compute instance batch actions to run concurrently"
    )]
    pub batch_concurrency: Option<usize>,

    #[arg(long, global = true, help = "Path to the TOML configuration file")]
    pub config: Option<PathBuf>,

    #[arg(
        long,
        global = true,
        help = "Path to the TOML plugin file; deliberately NOT derived from --config, \
                so a repository-supplied config file cannot bring executable declarations"
    )]
    pub plugins: Option<PathBuf>,

    #[arg(
        long,
        global = true,
        help = "Serve embedded JSON fixtures instead of calling gcloud, so every view renders offline"
    )]
    pub demo: bool,

    #[arg(
        long,
        global = true,
        help = "Refuse cloud-resource mutations (GCE/Cloud SQL lifecycle actions and SSH launches); \
                local actions such as activating a gcloud configuration and opening tunnels still work"
    )]
    pub read_only: bool,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Print config/log/cache/theme paths without starting the TUI.
    Info,
}

/// The fully resolved configuration: built-in defaults, folded over the
/// config file, folded over CLI flags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedConfig {
    pub starting_configuration: Option<String>,
    pub poll_interval_secs: u64,
    pub log_file: PathBuf,
    pub gcloud_binary: PathBuf,
    /// `bq` binary the BigQuery service shells to. CLI-only, like
    /// every other binary path here. BigQuery has no GA gcloud surface, so
    /// this is a second Cloud SDK binary rather than another gcloud argv.
    pub bq_binary: PathBuf,
    pub k9s_binary: PathBuf,
    pub kubectl_binary: PathBuf,
    /// Cloud SQL Auth Proxy binary a `CloudSqlProxy` managed tunnel spawns.
    /// CLI-only, like every other binary path here.
    pub cloud_sql_proxy_binary: PathBuf,
    pub config_path: PathBuf,
    /// Where user-declared plugins are read from.
    ///
    /// Resolved from the platform config directory or `--plugins`, and
    /// **never** from `--config`'s directory. This deliberately inverts the
    /// rule `config/state.rs` uses: state is stored beside the config file in
    /// use, because state is a file the app *writes*. This is a file the app
    /// *executes*, and locality is right for the first and wrong for the
    /// second — `nuvem --config ./repo.toml` must not be able to bring
    /// executable declarations with it.
    pub plugins_path: PathBuf,
    pub default_service: ResourceServiceKind,
    pub sidebar_visible: bool,
    pub theme: ThemeChoice,
    pub aliases: AliasTable,
    pub hotkeys: KeyOverrides,
    /// Validated `[columns.<service>]` blocks. Empty when the file
    /// has no `[columns]` table, which is what keeps the default tables
    /// byte-for-byte unchanged for an unconfigured user.
    pub columns: ColumnSettings,
    /// Validated `[[jumps]]` rules. The built-in set when the file
    /// has no `[[jumps]]` array, which is what keeps an unconfigured user's
    /// jump picker byte-for-byte unchanged.
    pub jumps: JumpRules,
    /// Validated `[[plugins]]` entries from the separate plugin file.
    /// Empty when that file is absent, which is what keeps an
    /// unconfigured user's app byte-for-byte unchanged.
    pub plugins: Plugins,
    /// Validated `[favorites]` table: favorited project ids, region names and
    /// zone names. Empty when the file has no `[favorites]` table, which is
    /// what keeps an unconfigured user's picker order byte-for-byte
    /// unchanged.
    pub favorites: FavoriteScopes,
    /// Validated `[theme.overrides]` and `[theme.project_overrides]` tables:
    /// a frame color per gcloud configuration and per project id.
    /// Empty when the file has no `[theme]` table, which is what
    /// keeps an unconfigured user's frame byte-for-byte unchanged.
    pub theme_overrides: ThemeOverrides,
    /// Maximum number of compute-instance batch actions run concurrently
    /// (issue #35). CLI-overridable; defaults to 4.
    pub batch_concurrency: usize,
    /// Demo fixture mode: serve embedded JSON fixtures through the
    /// normal service layer instead of shelling out to `gcloud`, so every view
    /// renders offline with no live GCP auth. CLI-only (`--demo`); off by
    /// default.
    pub demo: bool,
    /// Read-only mode: a promise about cloud-resource mutations
    /// only — GCE/Cloud SQL lifecycle actions and SSH launches are refused.
    /// It does not claim the app makes no local changes: activating a gcloud
    /// configuration, opening tunnels, and writing clipboard content still
    /// work.
    ///
    /// Settable from both `--read-only` and `[general] read_only` in the
    /// config file. The crate documents "flags take precedence over the
    /// config file", and this field looks like an exception — file-`true`
    /// survives even when `--read-only` is absent — but it isn't one:
    /// `--read-only` is a clap bool flag, so its absence means "not
    /// requested", never "force off". There is no way to express "off" on
    /// the command line, so there is nothing for a flag to override; file-on
    /// stays on. Both sides can only turn this on, never off, so the
    /// resolved value is the OR of the two, independent of fold order.
    pub read_only: bool,
}

impl Default for ResolvedConfig {
    fn default() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
        let cache_dir = dirs::cache_dir().unwrap_or_else(|| home.join(".cache"));
        Self {
            starting_configuration: None,
            poll_interval_secs: 5,
            log_file: cache_dir.join("nuvem").join("nuvem.log"),
            gcloud_binary: PathBuf::from("gcloud"),
            bq_binary: PathBuf::from("bq"),
            k9s_binary: PathBuf::from("k9s"),
            kubectl_binary: PathBuf::from("kubectl"),
            cloud_sql_proxy_binary: PathBuf::from("cloud-sql-proxy"),
            config_path: default_config_path(),
            plugins_path: default_plugins_path(),
            default_service: ResourceServiceKind::ComputeInstances,
            sidebar_visible: true,
            theme: ThemeChoice::default(),
            aliases: AliasTable::empty(),
            hotkeys: KeyOverrides::empty(),
            columns: ColumnSettings::empty(),
            jumps: JumpRules::defaults(),
            plugins: Plugins::empty(),
            favorites: FavoriteScopes::empty(),
            theme_overrides: ThemeOverrides::default(),
            batch_concurrency: 4,
            demo: false,
            read_only: false,
        }
    }
}

/// Back-compat alias: existing code refers to `Config`.
pub type Config = ResolvedConfig;

/// Result of [`ResolvedConfig::resolve`]: the resolved config plus any
/// non-fatal problems encountered while resolving it (bad file, unknown
/// version, unknown enum value, ...). Resolution never fails outright —
/// problems degrade to warnings and defaulted fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigOutcome {
    pub config: ResolvedConfig,
    pub problems: ConfigProblems,
    /// Set only when the config file itself failed to load (unreadable/bad
    /// TOML) or declared an unsupported version. Field-level problems (a bad
    /// alias, unknown theme, unknown default_service, ...) never set this —
    /// they stay in `problems` and the config still counts as "loaded".
    pub load_error: Option<String>,
}

/// The paths and active theme reported by `nuvem info`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigPaths {
    pub config: PathBuf,
    /// The separate plugin file, reported so a user can see which
    /// file nuvem actually read — it is deliberately not derived from
    /// `config`, so the two can name different directories.
    pub plugins: PathBuf,
    pub log: PathBuf,
    pub cache: PathBuf,
    pub theme: String,
}

impl ResolvedConfig {
    /// Resolve the effective configuration from built-in defaults, the
    /// config file (if any), and CLI flags, in that precedence order.
    /// Never returns an error: a missing, malformed, or invalid file
    /// degrades to warnings with the corresponding fields left at their
    /// defaults.
    pub fn resolve(cli: &Cli) -> ConfigOutcome {
        Self::resolve_over(cli, &ColumnSettings::empty())
    }

    /// [`Self::resolve`], but validating `[columns.<service>]` blocks
    /// against `running_columns` — the column settings currently in force —
    /// rather than assuming nothing is running yet. A live reload calls this
    /// directly so an invalidated service's warning can say it kept what was
    /// running instead of claiming a fallback that isn't true; a fresh load
    /// goes through `resolve`, which has nothing running and so passes
    /// [`ColumnSettings::empty()`].
    pub fn resolve_over(cli: &Cli, running_columns: &ColumnSettings) -> ConfigOutcome {
        let mut problems = ConfigProblems::default();
        let mut load_error = None;
        let mut config = ResolvedConfig::default();

        let path = cli.config.clone().unwrap_or_else(default_config_path);
        config.config_path = path.clone();
        config.plugins_path = cli.plugins.clone().unwrap_or_else(default_plugins_path);

        match file::load(&path) {
            Ok(Some(file_config)) => {
                if version_ok(file_config.version) {
                    apply_file(&mut config, &file_config, &mut problems, running_columns);
                } else {
                    load_error = Some(format!(
                        "{}: unsupported config version {:?}; falling back to defaults",
                        path.display(),
                        file_config.version
                    ));
                }
            }
            Ok(None) => {}
            Err(err) => load_error = Some(err.to_string()),
        }

        apply_cli(&mut config, cli);

        // After `apply_cli`, so `--plugins` has been folded in. A separate
        // load from a separate file: a problem here leaves everything the
        // config file resolved untouched.
        config.plugins = Plugins::load(&config.plugins_path, &mut problems);

        // Load-level failures are also recorded in `problems` for callers
        // (and tests) that only look at the warning list; `run_info` avoids
        // double-printing the same message as both an `error:` status and a
        // `warning:` line (see `info_report`).
        if let Some(err) = &load_error {
            problems.push(ConfigProblemSource::Load, err.clone());
        }

        ConfigOutcome {
            config,
            problems,
            load_error,
        }
    }

    /// Thin shim over [`Self::resolve`] for callers that only want the
    /// resolved config and don't care about problems.
    pub fn from_cli(cli: &Cli) -> Self {
        Self::resolve(cli).config
    }

    /// The config/log/cache paths and active theme name, as reported by
    /// `nuvem info`.
    pub fn paths(&self) -> ConfigPaths {
        let cache = self
            .log_file
            .parent()
            .map(PathBuf::from)
            .unwrap_or_else(|| dirs::cache_dir().unwrap_or_else(|| PathBuf::from("/tmp")));
        ConfigPaths {
            config: self.config_path.clone(),
            plugins: self.plugins_path.clone(),
            log: self.log_file.clone(),
            cache,
            theme: self.theme.name().to_string(),
        }
    }
}

/// Build the deterministic, labelled `nuvem info` report from a resolved
/// config outcome and a precomputed config status string (e.g. `"loaded"`,
/// `"not found"`, or `"error: ..."`).
pub fn info_report(outcome: &ConfigOutcome, config_status: &str) -> String {
    let paths = outcome.config.paths();
    let mut out = String::new();
    out.push_str(&format!("nuvem {}\n", env!("CARGO_PKG_VERSION")));
    out.push_str(&format!(
        "config:  {} ({})\n",
        paths.config.display(),
        config_status
    ));
    // `(0 loaded)` on its own cannot tell a user that nuvem never opened a
    // file — which is the report a macOS user gets after writing
    // `~/.config/nuvem/plugins.toml` while nuvem reads Application Support.
    // The status is computed here rather than passed in like `config_status`
    // because nothing but the path decides it: unlike the config file, a
    // plugin file that fails to load is a warning, never a `load_error`.
    let plugin_status = if paths.plugins.exists() {
        format!("{} loaded", outcome.config.plugins.len())
    } else {
        "not found".to_string()
    };
    out.push_str(&format!(
        "plugins: {} ({plugin_status})\n",
        paths.plugins.display(),
    ));
    out.push_str(&format!("log:     {}\n", paths.log.display()));
    out.push_str(&format!("cache:   {}\n", paths.cache.display()));
    out.push_str(&format!("theme:   {}\n", paths.theme));
    for warning in outcome.problems.messages() {
        // The load-level failure (if any) is already surfaced via
        // `config_status` as `error: ...`; skip it here so it isn't printed
        // twice.
        if outcome.load_error.as_deref() == Some(warning) {
            continue;
        }
        out.push_str(&format!("warning: {warning}\n"));
    }
    out
}

/// Resolve the config from `cli` and render the full `nuvem info` report:
/// config/log/cache/theme paths, without starting the TUI.
pub fn run_info(cli: &Cli) -> String {
    let outcome = ResolvedConfig::resolve(cli);

    let path = cli.config.clone().unwrap_or_else(default_config_path);
    let status = if !path.exists() {
        "not found".to_string()
    } else if let Some(err) = &outcome.load_error {
        format!("error: {err}")
    } else {
        "loaded".to_string()
    };

    info_report(&outcome, &status)
}

/// `dirs::config_dir()/nuvem/config.toml`, falling back to
/// `~/.config/nuvem/config.toml` if the platform config dir is unavailable.
pub fn default_config_path() -> PathBuf {
    let base = dirs::config_dir().unwrap_or_else(|| {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join(".config")
    });
    base.join("nuvem").join("config.toml")
}

/// `dirs::config_dir()/nuvem/plugins.toml`, falling back to
/// `~/.config/nuvem/plugins.toml` if the platform config dir is unavailable.
///
/// Takes no config path on purpose; see [`ResolvedConfig::plugins_path`].
///
/// Where `default_config_path` ends its fallback chain at `/tmp`, this one
/// refuses to: with neither a config dir nor a home dir there is no user
/// configuration directory, and `/tmp` is world-writable, so any other local
/// account could plant the file. That is harmless for `config.toml`, which
/// cannot run anything, and is the whole risk for this one. `/nonexistent` is
/// a path under a root-owned directory, so an unprivileged user cannot create
/// it: no config dir resolves to no plugins rather than to anyone's plugins.
pub fn default_plugins_path() -> PathBuf {
    let base = dirs::config_dir()
        .or_else(|| dirs::home_dir().map(|home| home.join(".config")))
        .unwrap_or_else(|| PathBuf::from("/nonexistent"));
    base.join("nuvem").join("plugins.toml")
}

fn version_ok(version: Option<u64>) -> bool {
    // Missing version is treated as version 1 for forward friendliness.
    matches!(version, None | Some(1))
}

fn apply_file(
    config: &mut ResolvedConfig,
    file_config: &file::FileConfig,
    problems: &mut ConfigProblems,
    running_columns: &ColumnSettings,
) {
    // Named before anything is applied, so a warning about a key that was
    // ignored appears above the settings that were not.
    for key in file_config.unknown.keys() {
        problems.push(
            ConfigProblemSource::Load,
            format!("unknown top-level setting {key:?}; ignoring it"),
        );
    }
    if let Some(general) = &file_config.general {
        for key in general.unknown.keys() {
            problems.push(
                ConfigProblemSource::General,
                format!("general: unknown setting {key:?}; ignoring it"),
            );
        }
        if let Some(poll_interval_secs) = general.poll_interval_secs {
            config.poll_interval_secs = poll_interval_secs;
        }
        if let Some(sidebar_visible) = general.sidebar_visible {
            config.sidebar_visible = sidebar_visible;
        }
        if let Some(default_service) = &general.default_service {
            match parse_service_kind(default_service) {
                Some(kind) => config.default_service = kind,
                None => problems.push(
                    ConfigProblemSource::General,
                    format!("unknown general.default_service {default_service:?}; using default"),
                ),
            }
        }
        if let Some(theme) = &general.theme {
            match ThemeChoice::from_str_lenient(theme) {
                Some(choice) => config.theme = choice,
                None => problems.push(
                    ConfigProblemSource::General,
                    format!("unknown general.theme {theme:?}; using default"),
                ),
            }
        }
        if let Some(read_only) = general.read_only
            && read_only
        {
            config.read_only = true;
        }
    }

    if let Some(raw_aliases) = &file_config.aliases {
        config.aliases = AliasTable::from_file(raw_aliases, problems);
    }

    if let Some(raw_hotkeys) = &file_config.hotkeys {
        match KeyOverrides::from_file(raw_hotkeys) {
            Ok(overrides) => config.hotkeys = overrides,
            Err(messages) => problems.extend(ConfigProblemSource::Hotkeys, messages),
        }
    }

    if let Some(raw_columns) = &file_config.columns {
        config.columns = ColumnSettings::from_file(raw_columns, problems, running_columns);
    }

    if let Some(raw_jumps) = &file_config.jumps {
        let mut messages = Vec::new();
        config.jumps = JumpRules::from_file(raw_jumps, &mut messages);
        problems.extend(ConfigProblemSource::Jumps, messages);
    }

    if let Some(raw_favorites) = &file_config.favorites {
        config.favorites = FavoriteScopes::from_file(raw_favorites, problems);
    }

    if let Some(raw_theme) = &file_config.theme {
        config.theme_overrides = ThemeOverrides::from_file(raw_theme, problems);
    }

    // Read to be refused, never to be used. Copying the schema into this file
    // is the one misplacement the two-file split makes likely, and it is the
    // misplacement the split exists to prevent — so it has to say so out loud
    // rather than leave the user with a config nuvem accepted and a feature
    // that never appears.
    if file_config.plugins.is_some() {
        problems.push(
            ConfigProblemSource::Plugins,
            format!(
                "plugins: plugin declarations belong in {}, not in the config file; \
                 ignoring them here",
                config.plugins_path.display()
            ),
        );
    }
}

fn apply_cli(config: &mut ResolvedConfig, cli: &Cli) {
    if let Some(configuration) = &cli.configuration {
        config.starting_configuration = Some(configuration.clone());
    }
    if let Some(poll_interval) = cli.poll_interval {
        config.poll_interval_secs = poll_interval;
    }
    if let Some(log_file) = &cli.log_file {
        config.log_file = log_file.clone();
    }
    if let Some(gcloud_binary) = &cli.gcloud_binary {
        config.gcloud_binary = gcloud_binary.clone();
    }
    if let Some(bq_binary) = &cli.bq_binary {
        config.bq_binary = bq_binary.clone();
    }
    if let Some(k9s_binary) = &cli.k9s_binary {
        config.k9s_binary = k9s_binary.clone();
    }
    if let Some(kubectl_binary) = &cli.kubectl_binary {
        config.kubectl_binary = kubectl_binary.clone();
    }
    if let Some(cloud_sql_proxy_binary) = &cli.cloud_sql_proxy_binary {
        config.cloud_sql_proxy_binary = cloud_sql_proxy_binary.clone();
    }
    if let Some(batch_concurrency) = cli.batch_concurrency {
        config.batch_concurrency = batch_concurrency;
    }
    if cli.demo {
        config.demo = true;
    }
    if cli.read_only {
        config.read_only = true;
    }
}

/// Resolve a config-file service value to a [`ServiceId`], leniently: known
/// short names first, then falling back to matching a built-in service label
/// or alias exactly.
///
/// This is the one service vocabulary the config file speaks: `[aliases]`
/// values and `[columns.<service>]` keys both go through it, so a service is
/// named the same way everywhere in the file.
pub(crate) fn resolve_service_id(raw: &str) -> Option<ServiceId> {
    let value = raw.trim().to_ascii_lowercase();
    match value.as_str() {
        "gce" | "compute" | "compute_instances" | "compute-instances" => {
            Some(ServiceId::ComputeInstances)
        }
        "gke" | "clusters" | "gke_clusters" | "gke-clusters" => Some(ServiceId::GkeClusters),
        "cloudsql" | "cloud_sql" | "cloud-sql" => Some(ServiceId::CloudSql),
        "run" | "cloud_run" | "cloud-run" | "cloudrun" => Some(ServiceId::CloudRun),
        _ => service_registry::service_descriptors()
            .iter()
            .find_map(|descriptor| {
                let matches_label = descriptor.label.eq_ignore_ascii_case(&value);
                let matches_alias = descriptor
                    .aliases
                    .iter()
                    .any(|alias| alias.eq_ignore_ascii_case(&value));
                (matches_label || matches_alias).then_some(descriptor.id)
            }),
    }
}

/// [`resolve_service_id`], narrowed to the resource service the id targets.
/// Every registered service targets a resource list today; the `and_then`
/// keeps that from being an assumption.
pub(crate) fn resolve_service_kind(raw: &str) -> Option<ResourceServiceKind> {
    resolve_service_id(raw)
        .and_then(|id| service_registry::descriptor(id).target)
        .and_then(ServiceTarget::resource_kind)
}

/// The name the config file writes a service under when the *file* is what
/// gets written — the `to` half of a `[[jumps]]` rule, which is generated from
/// [`crate::app::state::navigation::DetailTab`] rather than typed by hand, and
/// the value of a `[data.views]` entry in the state file
/// ([`crate::app::state::remembered_view`]).
///
/// The inverse of [`resolve_service_kind`], which accepts every spelling; this
/// picks the one to print. `config_service_id_round_trips` pins the two
/// together so a generated name is always a name the parser accepts.
pub(crate) fn config_service_id(kind: ResourceServiceKind) -> &'static str {
    match kind {
        ResourceServiceKind::ComputeInstances => "compute",
        ResourceServiceKind::GkeClusters => "gke",
        ResourceServiceKind::CloudSql => "cloudsql",
        ResourceServiceKind::CloudRunServices => "cloudrun",
        ResourceServiceKind::CloudStorage => "storage",
        ResourceServiceKind::SecretManagerSecrets => "secrets",
        ResourceServiceKind::CloudFunctions => "functions",
        ResourceServiceKind::PersistentDisks => "disks",
        ResourceServiceKind::PubSubTopics => "pubsub-topics",
        ResourceServiceKind::VpcNetworks => "vpc-networks",
        ResourceServiceKind::VpcSubnets => "vpc-subnets",
        ResourceServiceKind::FirewallRules => "firewall-rules",
        ResourceServiceKind::PubSubSubscriptions => "pubsub-subscriptions",
        ResourceServiceKind::BigQueryDatasets => "bigquery-datasets",
        ResourceServiceKind::IamServiceAccounts => "service-accounts",
        ResourceServiceKind::IamPolicy => "iam-policy",
    }
}

/// `general.default_service`'s own, narrower vocabulary — the startup service
/// has only ever accepted GCE and GKE. Distinct from [`resolve_service_kind`]
/// on purpose: widening it would change which service the app opens on when a
/// configuration has no remembered view yet.
fn parse_service_kind(raw: &str) -> Option<ResourceServiceKind> {
    match raw.to_ascii_lowercase().as_str() {
        "gce" | "compute" | "compute_instances" => Some(ResourceServiceKind::ComputeInstances),
        "gke" | "clusters" | "gke_clusters" => Some(ResourceServiceKind::GkeClusters),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cli(args: &[&str]) -> Cli {
        let mut full = vec!["nuvem"];
        full.extend_from_slice(args);
        Cli::parse_from(full)
    }

    #[test]
    fn defaults_apply_with_no_config_flag() {
        let out = ResolvedConfig::resolve(&cli(&["--config", "/nonexistent/nuvem.toml"]));
        assert_eq!(out.config.poll_interval_secs, 5);
        assert!(out.config.sidebar_visible);
        assert_eq!(
            out.config.default_service,
            ResourceServiceKind::ComputeInstances
        );
        assert!(out.problems.is_empty());
    }

    #[test]
    fn cli_flags_override_defaults() {
        let out = ResolvedConfig::resolve(&cli(&[
            "--config",
            "/nonexistent/nuvem.toml",
            "--poll-interval",
            "30",
            "--configuration",
            "dev",
        ]));
        assert_eq!(out.config.poll_interval_secs, 30);
        assert_eq!(out.config.starting_configuration.as_deref(), Some("dev"));
    }

    #[test]
    fn config_service_id_round_trips() {
        for kind in ResourceServiceKind::ALL {
            assert_eq!(resolve_service_kind(config_service_id(kind)), Some(kind));
        }
    }

    #[test]
    fn parse_service_kind_accepts_known_aliases() {
        assert_eq!(
            parse_service_kind("gce"),
            Some(ResourceServiceKind::ComputeInstances)
        );
        assert_eq!(
            parse_service_kind("compute_instances"),
            Some(ResourceServiceKind::ComputeInstances)
        );
        assert_eq!(
            parse_service_kind("gke"),
            Some(ResourceServiceKind::GkeClusters)
        );
        assert_eq!(parse_service_kind("nonsense"), None);
    }

    #[test]
    fn version_ok_accepts_missing_and_v1_only() {
        assert!(version_ok(None));
        assert!(version_ok(Some(1)));
        assert!(!version_ok(Some(2)));
    }

    #[test]
    fn defaults_use_k9s_and_kubectl_on_path() {
        let config = ResolvedConfig::default();
        assert_eq!(config.k9s_binary, PathBuf::from("k9s"));
        assert_eq!(config.kubectl_binary, PathBuf::from("kubectl"));
    }

    #[test]
    fn cloud_sql_proxy_binary_defaults_to_the_one_on_path() {
        assert_eq!(
            ResolvedConfig::default().cloud_sql_proxy_binary,
            PathBuf::from("cloud-sql-proxy")
        );
    }

    #[test]
    fn cloud_sql_proxy_binary_flag_overrides_the_default() {
        // Binaries are CLI-only in this codebase: there is no config-file key
        // to fold, so the flag is the only override path.
        let out = ResolvedConfig::resolve(&cli(&[
            "--config",
            "/nonexistent/nuvem.toml",
            "--cloud-sql-proxy-binary",
            "/opt/bin/cloud-sql-proxy",
        ]));
        assert_eq!(
            out.config.cloud_sql_proxy_binary,
            PathBuf::from("/opt/bin/cloud-sql-proxy")
        );
    }

    #[test]
    fn bq_binary_defaults_to_the_one_on_path() {
        assert_eq!(ResolvedConfig::default().bq_binary, PathBuf::from("bq"));
    }

    #[test]
    fn bq_binary_flag_overrides_the_default() {
        // Binaries are CLI-only in this codebase: there is no config-file key
        // to fold, so the flag is the only override path.
        let out = ResolvedConfig::resolve(&cli(&[
            "--config",
            "/nonexistent/nuvem.toml",
            "--bq-binary",
            "/opt/sdk/bin/bq",
        ]));
        assert_eq!(out.config.bq_binary, PathBuf::from("/opt/sdk/bin/bq"));
    }

    #[test]
    fn default_config_path_ends_with_nuvem_config_toml() {
        let path = default_config_path();
        assert!(path.ends_with("nuvem/config.toml"));
    }

    /// The plugin path is asserted by *shape* here, not by comparison against
    /// `default_plugins_path()`. The integration test can only compare the
    /// resolved value to this function, which is tautological — it holds
    /// equally well if the function returns nothing at all. This is the
    /// assertion that says what the path must be.
    #[test]
    fn default_plugins_path_ends_with_nuvem_plugins_toml() {
        let path = default_plugins_path();
        assert!(
            path.ends_with("nuvem/plugins.toml"),
            "{path:?} is not the platform plugin file"
        );
        // Beside `config.toml`, in the same directory, and never below it:
        // the two files are siblings even though only one of them follows
        // `--config`.
        assert_eq!(path.parent(), default_config_path().parent());
    }

    #[test]
    fn demo_flag_sets_demo_true() {
        let out = ResolvedConfig::resolve(&cli(&["--demo", "--config", "/nonexistent/nuvem.toml"]));
        assert!(out.config.demo);
    }

    #[test]
    fn demo_defaults_to_false() {
        let out = ResolvedConfig::resolve(&cli(&["--config", "/nonexistent/nuvem.toml"]));
        assert!(!out.config.demo);
    }

    #[test]
    fn read_only_defaults_to_false() {
        let out = ResolvedConfig::resolve(&cli(&["--config", "/nonexistent/nuvem.toml"]));
        assert!(!out.config.read_only);
    }

    #[test]
    fn read_only_flag_sets_read_only_true() {
        let out = ResolvedConfig::resolve(&cli(&[
            "--read-only",
            "--config",
            "/nonexistent/nuvem.toml",
        ]));
        assert!(out.config.read_only);
    }

    #[test]
    fn read_only_file_true_survives_absent_flag() {
        // The disagreement case: the file turns it on and no --read-only flag
        // is passed. A bare bool flag has no way to express "off" on the
        // command line, so its absence must never clear a file-on setting.
        let mut file = tempfile::NamedTempFile::new().unwrap();
        std::io::Write::write_all(&mut file, b"version = 1\n[general]\nread_only = true\n")
            .unwrap();

        let out = ResolvedConfig::resolve(&cli(&["--config", file.path().to_str().unwrap()]));
        assert!(out.config.read_only);
    }

    #[test]
    fn read_only_file_false_plus_flag_is_still_true() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        std::io::Write::write_all(&mut file, b"version = 1\n[general]\nread_only = false\n")
            .unwrap();

        let out = ResolvedConfig::resolve(&cli(&[
            "--read-only",
            "--config",
            file.path().to_str().unwrap(),
        ]));
        assert!(out.config.read_only);
    }
}
