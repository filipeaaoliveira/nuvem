pub mod clipboard;
pub mod interactive;
pub mod k9s;
pub mod log_export;
pub mod logging;
pub mod remembered_view_store;
pub mod session;
pub mod tail;
pub mod terminal;
pub mod tunnel;

use std::time::Duration;

use futures_util::StreamExt;
use ratatui::layout::Rect;

use crate::app::projection::binding::GLOBAL_HELP;
use crate::app::state::resources::PluginPrompt;
use crate::app::{AppState, NavigatorFocus, Workspace};
use crate::config::hotkeys::KeyOverrides;
use crate::config::problems::ConfigProblems;
use crate::config::theme::Theme;
use crate::input::{Context, Mode};
use crate::{Cli, Config, ConfigOutcome};

pub async fn run(outcome: ConfigOutcome, cli: Cli) -> anyhow::Result<()> {
    let ConfigOutcome {
        config,
        mut problems,
        load_error,
    } = outcome;
    logging::initialize_tracing(&config)?;
    if config.demo {
        tracing::info!("demo mode: serving embedded JSON fixtures instead of calling gcloud");
    }
    terminal::ensure_interactive_terminal()?;
    terminal::install_panic_restore_hook();

    crate::config::theme::set_active(Theme::for_choice(config.theme));

    let mut app = AppState::new();
    apply_startup_config(&mut app, &config);
    // Before `surface_startup_config_problems` below, so a state file that
    // could not be read reaches the same startup warning every other config
    // problem does.
    let startup_views = load_remembered_views(&config, &mut problems);
    app.root.remembered_views = startup_views.views;
    app.root.remembered_view_saves_suppressed = startup_views.saves_suppressed;
    let state_path = startup_views.state_path;

    // Captured before the config moves into the session: the problem pointer
    // has to name the help key this run is actually listening for.
    let hotkeys = config.hotkeys.clone();

    let mut session = session::RuntimeSession::new(config, cli, app, state_path);
    session.initialize().await;
    tracing::debug!(
        tmux = session.tmux_detected(),
        "interactive launch environment"
    );

    // `initialize()`'s own effect results (`ConfigurationsLoaded` /
    // `ConfigurationActivated`) clear the status message as part of settling
    // in, so the startup config warning/error must be surfaced only after it
    // returns, or it would be wiped before ever being drawn.
    surface_startup_config_problems(&mut session, &problems, load_error.as_deref(), &hotkeys);

    let mut terminal = ratatui::init();
    let result = run_loop(&mut terminal, &mut session).await;
    // The quit action flushes too; this covers the exits that never dispatch
    // one — a closed event stream, a closed result channel — so no run-loop
    // break can drop a save the writer still had in hand.
    session.flush_remembered_view_saves().await;
    // Same reason, for the same exits: a plugin still on a `spawn_blocking`
    // thread would otherwise hold `Runtime::drop` for as long as its argv wants.
    session.cancel_plugins_for_shutdown();
    // And the same for a lifecycle action — a batch of VM stops, or a single
    // row's failover. Its ceiling is thirty minutes, and without this
    // the operator's `q` restored the terminal, returned from here, and then
    // hung inside `Runtime::drop` with nothing on screen to say why.
    //
    // Before `ratatui::restore()`, not after: both cancellations are cheap flag
    // flips, and doing them while the terminal is still ours keeps the ordering
    // one thing — everything that could stall the exit is released first, then
    // the terminal goes back.
    session.cancel_lifecycle_actions_for_shutdown();
    ratatui::restore();
    result
}

/// Applies the safe/startup fields of the resolved config to a freshly
/// created [`AppState`], before the initial `initialize()` effects run.
/// `default_service` only changes which resources service is *active* by
/// default (it does not touch the starting configuration/workspace, which
/// `initialize()` handles separately), `sidebar_visible` sets the
/// service navigator's initial sidebar visibility, and the validated
/// `[columns.<service>]` settings are installed before any list loads, so the
/// first render of every table is already the configured one. The validated
/// `[[jumps]]` rules go in for the same reason: the `J` picker derives its rows
/// from them, so they have to be in place before the first `J`. The
/// `[favorites]` scopes go in for the same reason too: the configuration
/// context picker orders its rows by them, so they have to be in place
/// before the first time that picker opens. The validated `[[plugins]]`
/// entries go in here and *only* here: the command palette derives its plugin
/// entries from state, so they have to be installed before the first time it
/// opens — and a config reload deliberately leaves them alone, see
/// [`crate::app::state::root::RootState::plugins`].
pub fn apply_startup_config(app: &mut AppState, config: &Config) {
    app.root.aliases = config.aliases.clone();
    app.root.jump_rules = config.jumps.clone();
    app.root.plugins = config.plugins.clone();
    app.root.read_only = config.read_only;
    app.root
        .configurations
        .set_favorites(config.favorites.clone());
    // The remembered view for the starting configuration wins over
    // `default_service`, which is the fallback for a configuration with no
    // memory yet. The active configuration is not known until `initialize()`
    // resolves it, so this sets the fallback and the choke point in `reduce`
    // applies the remembered value when that configuration lands.
    app.root.resources.select_service(config.default_service);
    app.root.default_service = config.default_service;
    app.root
        .navigator
        .set_sidebar_visible(config.sidebar_visible);
    app.root
        .resources
        .set_column_settings(config.columns.clone());
}

/// What the startup load found: the map to start the session from, whether
/// this session is allowed to write the state file at all, and the file it
/// read — the one the session must write back to.
pub struct StartupRememberedViews {
    pub views: crate::app::state::remembered_view::RememberedViews,
    /// Seeds [`crate::app::state::root::RootState::remembered_view_saves_suppressed`].
    /// See that field for why an unreadable file must not be saved over.
    pub saves_suppressed: bool,
    /// The single derivation of the state path for the whole run: [`run`] hands
    /// this to [`session::RuntimeSession::new`] rather than letting the session
    /// derive its own, so load and save cannot address different files.
    pub state_path: std::path::PathBuf,
}

/// Reads the previous run's remembered-view map.
///
/// The state file is the sibling of the config file *in use*, so a
/// `--config <path>` override is honoured; `default_state_path` would ignore
/// it. A corrupt or future-version file degrades to an empty map plus a
/// `ConfigProblemSource::State` warning on `problems`, which the existing
/// startup surfacing pass already reports — which is why the caller must load
/// *before* [`surface_startup_config_problems`] runs or the warning would never
/// be drawn.
///
/// [`crate::config::state::load`] rather than its `load_or_default` sibling, for
/// one reason: `load_or_default` collapses "there is no file" and "there is a
/// file this build cannot read" into the same empty map, and those two must not
/// lead to the same behaviour — the second has to suppress saves so a state file
/// written by a newer nuvem is left intact. The warning pushed below extends
/// `load_or_default`'s wording with that suppression and the reason for it,
/// because a message that reported only the degraded read would leave the user
/// unaware that this run — and every run after it, since nothing rewrites the
/// file — has stopped remembering anything. It stops short of telling the user
/// to delete the file: in the very case the suppression protects, that advice
/// would destroy the newer build's remembered views.
///
/// Demo mode reads nothing: the fixture configurations are named `dev` and
/// `prod` like real ones, so a demo run that loaded the real file would open on
/// the service the user last had open for real, and a real file this build
/// cannot read would raise a startup warning in the middle of a demo. The
/// session withholds the store for the write half of the same isolation. The
/// path is still returned, because it is the run's single derivation and the
/// session takes it either way.
///
/// A function rather than two lines inlined in [`run`] so the startup restore
/// is reachable from a test: `run` itself needs a terminal.
pub fn load_remembered_views(
    config: &Config,
    problems: &mut ConfigProblems,
) -> StartupRememberedViews {
    use crate::app::state::remembered_view::RememberedViews;
    use crate::config::problems::ConfigProblemSource;

    let state_path = crate::config::state::state_path_for(&config.config_path);
    if config.demo {
        return StartupRememberedViews {
            views: RememberedViews::default(),
            saves_suppressed: false,
            state_path,
        };
    }
    match crate::config::state::load::<RememberedViews>(&state_path) {
        Ok(Some(views)) => StartupRememberedViews {
            views,
            saves_suppressed: false,
            state_path,
        },
        // No file yet: the ordinary first run. There is nothing to clobber, so
        // this session saves normally.
        Ok(None) => StartupRememberedViews {
            views: RememberedViews::default(),
            saves_suppressed: false,
            state_path,
        },
        Err(err) => {
            problems.push(
                ConfigProblemSource::State,
                // `state::load` composes the first half, and its
                // version-mismatch wording already ends by saying defaults are
                // in use, so this half only adds what that half cannot know:
                // that saving is off, and why. Deliberately no advice to remove
                // the file — that is the data loss the suppression prevents.
                format!(
                    "{}: {}; nuvem will not save the remembered view this session, in case a \
                     newer nuvem wrote that file",
                    err.path.display(),
                    err.message
                ),
            );
            StartupRememberedViews {
                views: RememberedViews::default(),
                saves_suppressed: true,
                state_path,
            }
        }
    }
}

/// How the status line tells the user where the full problem list lives: the
/// running help key when there is one to name, and a key-free phrasing when
/// there is not. Startup and reload both compose their multi-problem message
/// from this one function so the two can never drift apart.
pub fn help_pointer(hotkeys: &KeyOverrides) -> String {
    pointer_for_key(&crate::app::projection::binding::display_binding(&GLOBAL_HELP, hotkeys).key)
}

/// The pointer wording for an already-rendered help key. Split out from
/// [`help_pointer`] so the empty-key fallback — which no live binding produces
/// — is reachable from a test rather than asserted by inspection.
fn pointer_for_key(key: &str) -> String {
    if key.is_empty() {
        "open help for details".to_string()
    } else {
        format!("press {key} for details")
    }
}

/// The startup surfacing, in the one order that makes the deferred
/// `[theme.overrides]` check visible.
///
/// `initialize()` has already run that check against the configurations it
/// loaded, so the list the session holds is `resolve-time + deferred`. Seeding
/// the resolve-time half first rebuilds that merged list, and the status
/// summary is composed from it rather than from `problems` alone — otherwise a
/// config whose only problem is an override naming nothing would leave the
/// status line silent and the warning reachable only by opening help.
pub fn surface_startup_config_problems(
    session: &mut session::RuntimeSession,
    problems: &ConfigProblems,
    load_error: Option<&str>,
    hotkeys: &KeyOverrides,
) {
    session.set_base_config_problems(problems.messages().iter().map(|m| m.to_string()).collect());
    let merged = session.app().root.system.config_problems().to_vec();
    surface_config_problems(session.app_mut(), &merged, load_error, hotkeys);
}

/// Records every config problem on the app state (so the help overlay can
/// list them all) and surfaces a summary in the initial status message, so a
/// bad config is visible to the user without ever crashing the app.
///
/// The problems list is *replaced* on every call, including the empty case:
/// a clean load must never leave a previous load's problems on display. A
/// `load_error` (unreadable/unsupported-version file) takes precedence over
/// field-level problems in the status line, but the list is installed either
/// way.
pub fn surface_config_problems(
    app: &mut AppState,
    messages: &[String],
    load_error: Option<&str>,
    hotkeys: &KeyOverrides,
) {
    let summary = match messages {
        [] => None,
        [only] => Some(format!("config warning: {only}")),
        all => Some(format!(
            "config: {} problems ({})",
            all.len(),
            help_pointer(hotkeys)
        )),
    };

    app.root.system.set_config_problems(messages.to_vec());

    if let Some(error) = load_error {
        app.root
            .system
            .set_error_status_message(format!("config error: {error}"));
    } else if let Some(summary) = summary {
        app.root.system.set_status_message(summary);
    }
}

async fn run_loop(
    terminal: &mut ratatui::DefaultTerminal,
    session: &mut session::RuntimeSession,
) -> anyhow::Result<()> {
    let mut event_stream = crossterm::event::EventStream::new();
    let mut poll_interval = session.config().and_then(build_poll_interval);
    // Fast enough to animate the in-flight load spinner smoothly; a plain
    // 1s cadence made the spinner appear to stutter/freeze between frames.
    let mut redraw_interval = tokio::time::interval(Duration::from_millis(250));
    let mut termination = TerminationSignals::install();

    loop {
        terminal.draw(|frame| crate::ui::render(frame, session))?;

        tokio::select! {
            result = session.process_next_effect_result() => {
                if !result {
                    break;
                }
            }
            event = event_stream.next() => match event {
                Some(Ok(crossterm::event::Event::Key(key))) => {
                    let size = terminal.size()?;
                    let context = input_context(session, Rect::new(0, 0, size.width, size.height));
                    if let Some(action) = crate::input::keymap::action_for_key(key, &context) {
                        if !session.dispatch_action(action).await {
                            break;
                        }
                        run_pending_interactive(terminal, session).await;
                    }
                }
                Some(Ok(crossterm::event::Event::Resize(_, _))) => {}
                Some(Ok(_)) => {}
                Some(Err(error)) => return Err(error.into()),
                None => break,
            },
            _ = next_poll_tick(&mut poll_interval) => {
                session.poll_active_resource().await;
                session.poll_active_tail().await;
            }
            _ = termination.received() => {
                // Tunnel children are spawned into a session of their own, so
                // they never get the terminal's own `SIGHUP` and would survive
                // this process. Quitting is exactly what has to happen, so it
                // goes through the same path the `q` key does rather than
                // growing a second shutdown sequence to keep in step.
                tracing::info!("terminating on a signal: shutting managed tunnels down");
                session.dispatch_action(crate::app::Action::QuitRequested).await;
                break;
            }
            _ = redraw_interval.tick() => {}
        }

        // A runtime config reload may have changed the poll cadence; rebuild
        // the timer from the (updated) config when that happens.
        if session.take_poll_interval_dirty() {
            poll_interval = session.config().and_then(build_poll_interval);
        }
    }

    Ok(())
}

/// Runs the interactive command the session has stashed, if any: suspend the
/// TUI, hand the terminal to the child, resume, and fold the outcome back
/// into state. Public for the suite: `interactive_glue_evidence.rs` drives it
/// against a pseudo-terminal, because this is the one place the tested pieces
/// (take, resolve, execute, apply) are wired to the real lifecycle and a stub
/// here would launch nothing while every unit test stayed green.
pub async fn run_pending_interactive(
    terminal: &mut ratatui::DefaultTerminal,
    session: &mut session::RuntimeSession,
) {
    use crate::services::runtime::interactive::{
        RatatuiLifecycle, SubprocessRunner, execute_interactive_process,
    };
    let Some(command) = session.take_pending_interactive_command() else {
        return;
    };
    let spec = session.resolve_interactive_spec(&command);
    let description = spec.description.clone();
    let outcome = {
        let mut lifecycle = RatatuiLifecycle::new(terminal);
        execute_interactive_process(&mut lifecycle, &SubprocessRunner, &spec)
    };
    session
        .apply_interactive_outcome(description, outcome)
        .await;
}

fn build_poll_interval(config: &Config) -> Option<tokio::time::Interval> {
    (config.poll_interval_secs > 0).then(|| {
        let mut interval = tokio::time::interval(Duration::from_secs(config.poll_interval_secs));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        interval
    })
}

/// `SIGTERM`/`SIGHUP`, the one exit path that would otherwise leak tunnel
/// children.
///
/// Quit and a panic are both covered already (`shutdown_all_tunnels` and
/// `TunnelRegistry::drop`). A signal was not, and it used to be harmless: the
/// only children were foreground ones sharing the terminal's process group, so
/// the OS reaped them. Tunnel children are deliberately put in a new session
/// (`setsid`, so the whole group can be signalled), which also means the
/// terminal's `SIGHUP` never reaches them — closing the terminal with tunnels
/// open would strand them holding loopback ports.
///
/// `SIGINT` is deliberately absent: `Ctrl-C` arrives as a key event through the
/// terminal in raw mode and is already handled by the keymap.
struct TerminationSignals {
    #[cfg(unix)]
    signals: Option<(tokio::signal::unix::Signal, tokio::signal::unix::Signal)>,
}

impl TerminationSignals {
    /// Installs the handlers, or logs and carries on: a backstop that could not
    /// be installed must not stop the app from starting.
    #[cfg(unix)]
    fn install() -> Self {
        use tokio::signal::unix::{SignalKind, signal};
        let signals = match (
            signal(SignalKind::terminate()),
            signal(SignalKind::hangup()),
        ) {
            (Ok(terminate), Ok(hangup)) => Some((terminate, hangup)),
            (terminate, hangup) => {
                let error = terminate.err().or_else(|| hangup.err());
                tracing::warn!(
                    ?error,
                    "could not install the SIGTERM/SIGHUP handler; tunnels will only be \
                     shut down on quit"
                );
                None
            }
        };
        Self { signals }
    }

    #[cfg(not(unix))]
    fn install() -> Self {
        Self {}
    }

    /// Resolves when either signal arrives, and never otherwise — including
    /// when no handler could be installed, so the `select!` arm simply never
    /// wins instead of spinning.
    #[cfg(unix)]
    async fn received(&mut self) {
        let Some((terminate, hangup)) = self.signals.as_mut() else {
            return std::future::pending().await;
        };
        tokio::select! {
            _ = terminate.recv() => {}
            _ = hangup.recv() => {}
        }
    }

    #[cfg(not(unix))]
    async fn received(&mut self) {
        std::future::pending().await
    }
}

async fn next_poll_tick(interval: &mut Option<tokio::time::Interval>) {
    match interval {
        Some(interval) => {
            interval.tick().await;
        }
        None => std::future::pending::<()>().await,
    }
}

fn input_context(session: &session::RuntimeSession, terminal_area: Rect) -> Context {
    let command_palette_open = session.app().root.command.is_open();
    let configuration_activation_confirmation_open = session
        .app()
        .root
        .configurations
        .activation_confirmation()
        .is_some();
    let configuration_activation_confirmation_preview_visible = session
        .app()
        .root
        .configurations
        .activation_confirmation()
        .is_some_and(|confirmation| {
            crate::ui::configuration_activation_confirmation_preview_visible(
                terminal_area,
                confirmation,
            )
        });
    let resource_action_confirmation_open =
        session.app().root.resources.action_confirmation().is_some();
    let resource_action_confirmation_preview_visible = session
        .app()
        .root
        .resources
        .action_confirmation()
        .is_some_and(|confirmation| {
            crate::ui::resource_action_confirmation_preview_visible(terminal_area, confirmation)
        });
    let resource_batch_confirmation_open =
        session.app().root.resources.batch_confirmation().is_some();
    let resource_batch_confirmation_visible = session
        .app()
        .root
        .resources
        .batch_confirmation()
        .is_some_and(|confirmation| {
            crate::ui::resource_batch_confirmation_preview_visible(terminal_area, confirmation)
        });
    let resource_batch_ack_satisfied = session
        .app()
        .root
        .resources
        .batch_confirmation()
        .is_some_and(|confirmation| {
            confirmation
                .ack()
                .is_satisfied(confirmation.eligible().len())
        });
    let plugin_confirmation_open = session.app().root.resources.plugin_confirmation().is_some();
    // False when nothing is open, deliberately: the keymap's dispatch arm
    // requires `plugin_confirmation_open` and the `Enter` guard one arm deeper
    // reads this on its own, so a flag that defaulted to true would make the
    // pair's meaning depend on which of the two is read first.
    let plugin_confirmation_ack_satisfied = session
        .app()
        .root
        .resources
        .plugin_confirmation()
        .is_some_and(|confirmation| confirmation.is_ack_satisfied());
    let plugin_confirmation_visible = session
        .app()
        .root
        .resources
        .plugin_confirmation()
        .is_some_and(|confirmation| {
            crate::ui::plugin_confirmation_visible(terminal_area, confirmation)
        });
    let plugin_prompt_open = session.app().root.resources.plugin_prompt().is_some();
    // False when nothing is open, for the reason
    // `plugin_confirmation_ack_satisfied` is: the dispatch arm requires
    // `plugin_prompt_open` and the `Enter` guard one arm deeper reads this on
    // its own.
    let plugin_prompt_satisfied = session
        .app()
        .root
        .resources
        .plugin_prompt()
        .is_some_and(PluginPrompt::is_satisfied);
    let plugin_prompt_visible = session
        .app()
        .root
        .resources
        .plugin_prompt()
        .is_some_and(|prompt| crate::ui::plugin_prompt_visible(terminal_area, prompt));
    let resource_command_preview_open = session.app().root.resources.command_preview().is_some();
    let resource_command_preview_visible = session
        .app()
        .root
        .resources
        .command_preview()
        .is_some_and(|preview| crate::ui::resource_command_preview_visible(terminal_area, preview));
    let ssh_launch_preview_open = session.app().root.resources.ssh_launch_preview().is_some();
    let ssh_launch_preview_visible = session
        .app()
        .root
        .resources
        .ssh_launch_preview()
        .is_some_and(|preview| crate::ui::ssh_launch_preview_visible(terminal_area, preview));
    let tunnel_launch_preview_open = session
        .app()
        .root
        .resources
        .tunnel_launch_preview()
        .is_some();
    let tunnel_launch_preview_visible = session
        .app()
        .root
        .resources
        .tunnel_launch_preview()
        .is_some_and(|preview| crate::ui::tunnel_launch_preview_visible(terminal_area, preview));
    let k9s_launch_confirmation_open = session
        .app()
        .root
        .resources
        .k9s_launch_confirmation()
        .is_some();
    let k9s_launch_confirmation_preview_visible = session
        .app()
        .root
        .resources
        .k9s_launch_confirmation()
        .is_some_and(|confirmation| {
            crate::ui::k9s_launch_confirmation_preview_visible(terminal_area, confirmation)
        });
    let workspace = session.app().root.workspace;
    let service_sidebar_visible = session.app().root.navigator.sidebar_visible();
    let service_navigator_focused = workspace == Workspace::Home
        || (workspace == Workspace::Resources
            && service_sidebar_visible
            && session.app().root.navigator.focus() == NavigatorFocus::Navigator);
    let service_filter_open = session.app().root.navigator.is_filter_input_active()
        && (workspace == Workspace::Home
            || (workspace == Workspace::Resources && service_navigator_focused));
    let resources_filter_open =
        workspace == Workspace::Resources && session.app().root.resources.is_filter_input_active();
    let logging_filter_open =
        workspace == Workspace::Logging && session.app().root.logging.filter_active;
    let logging_detail_open =
        workspace == Workspace::Logging && session.app().root.logging.detail_open;
    let logging_filter_draft = session.app().root.logging.filter_draft.clone();
    let configurations_filter_open = workspace == Workspace::Configurations
        && session.app().root.configurations.is_filter_input_active();
    let configuration_create_open = workspace == Workspace::Configurations
        && session.app().root.configurations.is_create_form_open();
    let configuration_context_edit_open = workspace == Workspace::Configurations
        && session
            .app()
            .root
            .configurations
            .is_context_edit_form_open();
    let configuration_context_picker_open = session.app().root.workspace
        == crate::app::Workspace::Configurations
        && session.app().root.configurations.context_picker().is_some();
    let configuration_context_picker_filter_open = configuration_context_picker_open
        && session
            .app()
            .root
            .configurations
            .context_picker()
            .is_some_and(|picker| picker.is_filter_input_active());
    let configuration_context_picker_filter_draft = session
        .app()
        .root
        .configurations
        .context_picker()
        .map(|picker| picker.filter_draft().to_string())
        .unwrap_or_default();

    // Not gated on the workspace: `J` only binds in the resource views, so the
    // picker cannot be open anywhere else, and gating here would strand a modal
    // on screen with nothing dispatching its keys.
    let resource_jump_picker_open = session.app().root.jump_picker.is_some();

    // Armed at all, whatever it was armed over: a prefix whose scope has since
    // changed still owns the keystroke, and the reducer is what refuses it.
    // Dropping back to `Mode::Normal` on a mismatch would instead leave the flag
    // set with nothing to consume it, and re-arm the gesture the moment the
    // operator navigated back to that service.
    let plugin_prefix_pending = session.app().root.plugin_prefix_pending.is_some();

    let plugin_output_open = session.app().root.system.plugin_output().is_some();

    let mode = if session.app().root.system.show_help {
        Mode::Help
    } else if plugin_output_open {
        // Directly below help and above every confirmation, matching
        // `render_active_overlay`: the viewer is opened from the palette,
        // which closes as it executes, so nothing else can be open under it —
        // and whichever overlay renders is the one that must own the keys.
        Mode::PluginOutput
    } else if resource_action_confirmation_open || resource_batch_confirmation_open {
        Mode::ConfirmResourceAction
    } else if configuration_activation_confirmation_open {
        Mode::ConfirmConfigurationActivation
    } else if plugin_prompt_open {
        // Immediately above the confirmation it feeds, matching
        // `render_active_overlay`. The two are never open at once — the prompt
        // is taken as the confirmation is shown — so this rank is not about
        // resolving an overlap; it is what makes the prompt exclusive against
        // everything the confirmation is already exclusive against. The
        // `ConfirmPlugin` arm's `read_only` argument depends on that: the
        // refusal happens once, in `open_plugin`, and holds until the plugin
        // runs only because no key between the two can reach the command
        // palette, which is the only door to `ReloadConfig`. With the prompt
        // ranked here, its keys route to `plugin_prompt_key` and that stays
        // true across the new modal as well.
        Mode::PluginPrompt
    } else if plugin_confirmation_open {
        // Ahead of the command preview and matching `render_active_overlay`:
        // whichever overlay renders is the one that must own the keys. A
        // confirmation outranks the copy-only preview because its `Enter` runs
        // the command rather than copying it.
        Mode::PluginConfirmation
    } else if resource_command_preview_open {
        Mode::ResourceCommandPreview
    } else if ssh_launch_preview_open {
        Mode::SshLaunchPreview
    } else if k9s_launch_confirmation_open {
        // Deliberately ahead of the tunnel preview, matching
        // `render_active_overlay`'s precedence: whichever overlay renders is
        // the one that must own the keys.
        Mode::ConfirmK9sLaunch
    } else if tunnel_launch_preview_open {
        Mode::TunnelLaunchPreview
    } else if resource_jump_picker_open {
        // Same rank as its overlay in `render_active_overlay`: whichever modal
        // renders is the one that owns the keys.
        Mode::ResourceJumpPicker
    } else if command_palette_open {
        Mode::CommandPalette
    } else if configuration_context_picker_open {
        Mode::ConfigurationContextPicker
    } else if service_filter_open
        || resources_filter_open
        || configurations_filter_open
        || logging_filter_open
    {
        Mode::Filter
    } else if configuration_create_open || configuration_context_edit_open {
        Mode::EditConfiguration
    } else if plugin_prefix_pending {
        // Last before `Normal`, unlike every arm above it: those rank by which
        // overlay renders, and this mode renders none. `,` binds only in the
        // normal resource views, so no modal can be open when it arms — and
        // ranking it above one would strand that modal's keys if a future path
        // ever armed the prefix underneath it.
        Mode::PluginPrefix
    } else {
        Mode::Normal
    };

    Context {
        mode,
        workspace,
        resources_detail_open: session.app().root.resources.is_showing_detail(),
        resources_drilldown_open: session.app().root.resources.is_showing_drilldown(),
        resources_drilldown_opens_another_level: session
            .app()
            .root
            .resources
            .drilldown_opens_another_level(),
        service_filter_open,
        service_sidebar_visible,
        service_navigator_focused,
        resources_filter_open,
        configurations_filter_open,
        configurations_filter_draft: session.app().root.configurations.filter_draft().to_string(),
        configuration_create_open,
        configuration_context_edit_open,
        configuration_activation_confirmation_open,
        configuration_activation_confirmation_preview_visible,
        configuration_context_picker_open,
        configuration_context_picker_filter_open,
        configuration_context_picker_filter_draft,
        command_palette_open,
        resource_action_confirmation_open,
        resource_action_confirmation_preview_visible,
        resource_batch_confirmation_open,
        resource_batch_confirmation_visible,
        resource_batch_ack_satisfied,
        resource_command_preview_open,
        resource_command_preview_visible,
        ssh_launch_preview_open,
        ssh_launch_preview_visible,
        tunnel_launch_preview_open,
        tunnel_launch_preview_visible,
        logging_filter_open,
        logging_detail_open,
        logging_filter_draft,
        k9s_launch_confirmation_open,
        k9s_launch_confirmation_preview_visible,
        resource_jump_picker_open,
        plugin_confirmation_open,
        plugin_confirmation_ack_satisfied,
        plugin_confirmation_visible,
        plugin_prompt_open,
        plugin_prompt_satisfied,
        plugin_prompt_visible,
        plugin_output_open,
        hotkeys: session
            .config()
            .map(|config| config.hotkeys.clone())
            .unwrap_or_default(),
    }
}

pub fn input_context_for_test(session: &session::RuntimeSession, terminal_area: Rect) -> Context {
    input_context(session, terminal_area)
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use super::TerminationSignals;
    use super::{apply_startup_config, build_poll_interval, pointer_for_key};
    use crate::Config;
    use crate::app::AppState;
    use nuvem_domain::gcp::resource::ResourceServiceKind;
    use std::time::Duration;

    fn config_with_interval(secs: u64) -> Config {
        Config {
            poll_interval_secs: secs,
            ..Config::default()
        }
    }

    #[test]
    fn a_named_help_key_is_pointed_at_by_name() {
        assert_eq!(pointer_for_key("?"), "press ? for details");
    }

    /// `effective_trigger` always yields a trigger today, so this fallback is
    /// unreachable through `help_pointer` — which is the reason it is tested
    /// here. A status line reading "press  for details" would be worse than
    /// one that never claims a key at all.
    #[test]
    fn a_help_key_that_renders_empty_degrades_to_a_key_free_pointer() {
        assert_eq!(pointer_for_key(""), "open help for details");
    }

    #[test]
    fn apply_startup_config_carries_read_only_into_root_state() {
        let mut app = AppState::new();
        let config = Config {
            read_only: true,
            ..Config::default()
        };

        apply_startup_config(&mut app, &config);

        assert!(app.root.read_only);
    }

    #[test]
    fn apply_startup_config_stores_default_service_for_the_reducers_fallback() {
        let mut app = AppState::new();
        let config = Config {
            default_service: ResourceServiceKind::GkeClusters,
            ..Config::default()
        };

        apply_startup_config(&mut app, &config);

        assert_eq!(
            app.root.default_service,
            ResourceServiceKind::GkeClusters,
            "the reducer needs default_service in state to fall back to it on a \
             configuration with no remembered view"
        );
    }

    /// A plugin set built the only way a plugin is ever built: through
    /// [`crate::config::plugins::Plugins::load`] over a real file. A fixture
    /// that took a shortcut past validation would be installing a `Plugin` the
    /// config layer would never hand over.
    fn plugins_fixture_with_one_compute_entry() -> crate::config::plugins::Plugins {
        let mut file = tempfile::NamedTempFile::new().expect("a temp plugin file");
        std::io::Write::write_all(
            &mut file,
            b"version = 1\n\n[[plugins]]\nid = \"describe\"\nscope = \"compute\"\n\
              key = \"d\"\nlabel = \"Describe {name}\"\nargs = [\"compute\", \"instances\", \
              \"describe\", \"{name}\"]\n",
        )
        .expect("the fixture writes");

        let mut problems = crate::config::problems::ConfigProblems::default();
        let plugins = crate::config::plugins::Plugins::load(file.path(), &mut problems);
        assert!(
            problems.is_empty(),
            "fixture must be a valid plugin file: {:?}",
            problems.messages()
        );
        plugins
    }

    /// The palette derives its plugin entries from `RootState`, so the
    /// validated set has to be in place before the first time the palette
    /// opens — the same reason `[[jumps]]` is installed here.
    #[test]
    fn startup_installs_the_validated_plugin_set() {
        let mut app = AppState::new();
        let config = Config {
            plugins: plugins_fixture_with_one_compute_entry(),
            ..Config::default()
        };

        apply_startup_config(&mut app, &config);

        assert_eq!(app.root.plugins.len(), 1);
    }

    #[test]
    fn poll_interval_is_disabled_when_zero() {
        assert!(build_poll_interval(&config_with_interval(0)).is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn poll_interval_ticks_after_the_configured_delay() {
        let mut interval = build_poll_interval(&config_with_interval(5)).expect("interval enabled");
        interval.tick().await; // first tick fires immediately
        let start = tokio::time::Instant::now();
        interval.tick().await; // waits one virtual period
        assert_eq!(start.elapsed(), Duration::from_secs(5));
    }

    /// The degradation path: when the handlers could not be installed the
    /// backstop is simply absent, and `received()` must never resolve — a
    /// `select!` arm that completed immediately would spin the run loop into a
    /// busy quit instead of quietly never winning.
    ///
    /// The timeout is a "does not fire" bound, not a timing budget: nothing
    /// here is expected to take any time at all, so the value only decides how
    /// long the test spends proving a future stayed pending.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_backstop_that_could_not_be_installed_never_fires() {
        let mut termination = TerminationSignals { signals: None };

        let outcome = tokio::time::timeout(Duration::from_millis(50), termination.received()).await;

        assert!(
            outcome.is_err(),
            "a missing signal handler must leave `received()` pending forever"
        );
    }
}
