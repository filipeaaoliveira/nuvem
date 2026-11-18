use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use nuvem_domain::gcp::logging::LogTimeRange;

use crate::app::projection::binding::KeyTrigger;
use crate::app::{
    Action, CommandAction, ConfigurationsAction, JumpsAction, LoggingAction, NavAction,
    NavigatorAction, ResourcesAction, SystemAction, TunnelsAction, Workspace, WorkspaceAction,
};

use super::{Context, Mode};

pub fn action_for_key(key: KeyEvent, context: &Context) -> Option<Action> {
    if key.kind != KeyEventKind::Press {
        return None;
    }

    // Exactly CONTROL, not `.contains`: a `.contains` guard would also force-quit
    // on `Ctrl-Alt-C` and `Ctrl-Shift-C`, neither of which the app binds or the
    // help catalog claims.
    if key.code == KeyCode::Char('c') && key.modifiers == KeyModifiers::CONTROL {
        return Some(Action::QuitRequested);
    }

    // `Ctrl-x` cancels every in-flight plugin run, and it resolves
    // here beside `Ctrl-C` rather than in `normal_key` for the reason
    // `Ctrl-C` does: both are escapes from a process the UI no longer controls,
    // so neither can be scoped to the view that started it. A plugin outlives
    // the confirmation it was launched from, so a modal opened afterwards would
    // otherwise strand a run that never returns — sharpest under
    // `Mode::PluginPrefix`, which renders nothing that would explain the dead
    // key.
    //
    // Nothing is shadowed by resolving it this early. Every character arm below
    // is either gated on `is_plain_text_key` — which is `modifiers.is_empty()
    // || modifiers == SHIFT`, so `CONTROL` can never satisfy it — or matches an
    // exact `Ctrl-<char>` chord that is not `x`: today `Ctrl-t` and `Ctrl-S` in
    // normal-mode handlers, and `Ctrl-space` in `configuration_context_edit_key`,
    // which is the one modal handler binding a chord at all. So no mode bound
    // `Ctrl-x` or read it as a literal `x`; each answered `None`. Adding a
    // `Ctrl-x` arm anywhere below would now be dead code, because this returns
    // first. `CtrlChar('x')` is also a reserved trigger
    // (`config::hotkeys::reserved_reason`), so resolving ahead of the override
    // table in `normal_key` cannot take a key a config was allowed to claim.
    //
    // Matched exactly rather than with `.contains` for the reason `Ctrl-C` is:
    // `.contains` would hand `Ctrl-Alt-x` and `Ctrl-Shift-x` a destructive
    // action the catalog never claims. A bare `x` is deliberately left unbound —
    // the modifier is what keeps a stray keystroke from killing the operator's
    // command.
    if key.code == KeyCode::Char('x') && key.modifiers == KeyModifiers::CONTROL {
        return Some(Action::System(SystemAction::CancelRunningPlugins));
    }

    match context.mode {
        Mode::Normal => normal_key(key, context),
        // The only guards left in this match, and each is one conjunct where it
        // used to be two or three. `Mode::Filter` is assigned from a
        // four-way disjunction, so a guard is needed to say *which* filter is
        // up — but the `workspace ==` that sat beside each flag was not, because
        // `input_context` builds those flags as `workspace == W && ...` and the
        // conjunct restated the flag's own definition.
        //
        // The last arm of each group carries no guard: reaching it means every
        // earlier arm's flag was false, and the mode implies the disjunction, so
        // the remaining disjunct is what is left. The diff-scoped mutation gate
        // is what found these last two: `replace match guard ... with true`
        // survived, because no test can distinguish a guard that is never
        // false.
        //
        // These were the last two modes still able to reach the `modal_key`
        // fallback — the other eight lost their path when their own guards
        // went — so with them unguarded the fallback arm was unreachable, and
        // `modal_key`, whose only caller it was, went with it.
        Mode::Filter if context.service_filter_open => service_filter_key(key),
        Mode::Filter if context.resources_filter_open => resource_filter_key(key),
        Mode::Filter if context.configurations_filter_open => {
            configuration_filter_key(key, context)
        }
        Mode::Filter => logging_filter_key(key),
        // Same again, and the context-edit arm shed a third conjunct:
        // `!configuration_context_picker_open` restated the ladder's own
        // ordering, which tests the picker before it reaches this mode at all.
        Mode::EditConfiguration if context.configuration_create_open => {
            configuration_create_key(key)
        }
        Mode::EditConfiguration => configuration_context_edit_key(key),
        // Unguarded, like every arm from here down whose mode names one overlay.
        // A guard restating the `_open` flag would restate an
        // invariant the mode already carries: `services::runtime::input_context`
        // assigns each of these modes in exactly one place, and only under that
        // same flag — two flags for `ConfirmResourceAction`, whose branch is a
        // disjunction and whose guard was that same disjunction. So the guard
        // could never be false when the arm matched. It was dead.
        //
        // Removed because they were unreachable, not because the invariant
        // stopped mattering. If a mode ever gains a second assignment, its
        // guard has to come back with it.
        //
        // `keymap_mode_implication_property_test` states the invariant over
        // generated sessions, but read what that buys before leaning on it: a
        // second assignment fails a test only in a state those steps reach. A
        // branch on a condition the generator never produces is invisible
        // there, exactly as it is to the diff-scoped mutation gate. The cost
        // this accepts is that a broken ladder now dispatches into a handler
        // whose overlay is shut, where before the guard declined and `Esc`
        // cleared the overlay — so widen the generator when you add a branch,
        // rather than assuming it already covers you.
        //
        // The first of these was found the hard way, on
        // `Mode::PluginConfirmation`: the mutation gate flagged the guard as an
        // equivalent mutant, no test could kill it, and deleting the expression
        // was the only route to green. A later sweep took the rest pre-emptively
        // rather than paying that investigation once per branch.
        Mode::CommandPalette => command_palette_key(key),
        Mode::ConfigurationContextPicker => configuration_context_picker_key(key, context),
        Mode::ConfirmResourceAction => resource_action_confirmation_key(key, context),
        // Directly above the confirmation it feeds, and above everything the
        // confirmation outranks: the two are never open together — the prompt
        // is taken as the confirmation is shown — so their relative order is a
        // statement about the `input_context` ladder, which is where the
        // exclusivity that `ConfirmPlugin`'s `read_only` comment rests on is
        // established. Ranking the prompt here is what keeps every key routed
        // to `plugin_prompt_key` while it is open, so the command palette —
        // the only door to `ReloadConfig`, and so the only way `read_only` can
        // change — cannot be reached from under it.
        Mode::PluginPrompt => plugin_prompt_key(key, context),
        Mode::PluginConfirmation => plugin_confirmation_key(key, context),
        Mode::PluginPrefix => plugin_prefix_key(key),
        Mode::ResourceCommandPreview => resource_command_preview_key(key, context),
        Mode::SshLaunchPreview => ssh_launch_preview_key(key, context),
        Mode::TunnelLaunchPreview => tunnel_launch_preview_key(key, context),
        Mode::ConfirmK9sLaunch => k9s_launch_confirmation_key(key, context),
        // While the picker is up it owns `j`/`k`/`Enter`/`Esc`, which all mean
        // something else in the resource list behind it. That used to be a
        // statement about this arm's position; since every mode now appears
        // exactly once and `Mode::Normal` is the first arm, it is a statement
        // about the `input_context` ladder ranking the picker above `Normal`.
        Mode::ResourceJumpPicker => jump_picker_key(key),
        Mode::PluginOutput => plugin_output_key(key),
        Mode::Help => help_key(key),
        Mode::ConfirmConfigurationActivation => confirm_configuration_key(key, context),
    }
}

/// The plugin output viewer's keys.
///
/// `n`/`p` move between the runs the log still holds rather than scrolling,
/// which is the pair k9s uses for the same "next/previous of the same thing"
/// gesture. `y` copies, matching the copy key every preview modal already
/// binds, and is the escape hatch for output the retention bound cut.
fn plugin_output_key(key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Char('j') if is_plain_text_key(key) => {
            Some(Action::System(SystemAction::ScrollPluginOutputDown))
        }
        KeyCode::Down => Some(Action::System(SystemAction::ScrollPluginOutputDown)),
        KeyCode::Char('k') if is_plain_text_key(key) => {
            Some(Action::System(SystemAction::ScrollPluginOutputUp))
        }
        KeyCode::Up => Some(Action::System(SystemAction::ScrollPluginOutputUp)),
        KeyCode::PageDown => Some(Action::System(SystemAction::ScrollPluginOutputPageDown)),
        KeyCode::PageUp => Some(Action::System(SystemAction::ScrollPluginOutputPageUp)),
        KeyCode::Char('n') if is_plain_text_key(key) => {
            Some(Action::System(SystemAction::PluginOutputOlder))
        }
        KeyCode::Char('p') if is_plain_text_key(key) => {
            Some(Action::System(SystemAction::PluginOutputNewer))
        }
        KeyCode::Char('y') if is_plain_text_key(key) => {
            Some(Action::System(SystemAction::CopyPluginOutput))
        }
        KeyCode::Esc => Some(Action::System(SystemAction::ClosePluginOutput)),
        _ => None,
    }
}

/// The related-resource jump picker's keys. It has no filter and no
/// retry — its rows are derived from state that is already loaded — so it is
/// the smallest of the pickers: move, choose, leave.
fn jump_picker_key(key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Char('j') if is_plain_text_key(key) => {
            Some(Action::Jumps(JumpsAction::SelectNext))
        }
        KeyCode::Down => Some(Action::Jumps(JumpsAction::SelectNext)),
        KeyCode::Char('k') if is_plain_text_key(key) => {
            Some(Action::Jumps(JumpsAction::SelectPrevious))
        }
        KeyCode::Up => Some(Action::Jumps(JumpsAction::SelectPrevious)),
        KeyCode::Enter => Some(Action::Jumps(JumpsAction::Submit)),
        KeyCode::Esc => Some(Action::Jumps(JumpsAction::Cancel)),
        _ => None,
    }
}

fn normal_key(key: KeyEvent, context: &Context) -> Option<Action> {
    // Browser-style history traversal (Normal and detail contexts). Exactly
    // ALT: a `.contains` guard would also navigate history on `Ctrl-Alt-Left`
    // and `Alt-Shift-Left`, chords the help catalog never claims.
    if key.modifiers == KeyModifiers::ALT {
        match key.code {
            KeyCode::Left => return Some(Action::Navigation(NavAction::Back)),
            KeyCode::Right => return Some(Action::Navigation(NavAction::Forward)),
            _ => {}
        }
    }

    // Rebindable normal-mode global/navigation keys (home, configurations,
    // resources, refresh, help, quit, command_palette) resolve EXCLUSIVELY
    // through the effective override table (built-in defaults with any user
    // overrides applied). They are intentionally absent from the hardcoded
    // arms below so that a default key does not still trigger its old action
    // after being reassigned. `Ctrl-C` force-quit stays reserved/hardcoded in
    // `action_for_key`.
    if let Some(trigger) = override_trigger(key)
        && let Some(action) = context.hotkeys.action_for(trigger)
    {
        return Some(action.to_app_action(context));
    }

    match key.code {
        // A drill-down claims `Esc` before history does: inside either of
        // BigQuery's drilled views the operator's "back" is the list they
        // entered from — the datasets list from the tables view, the tables
        // list from a table's schema — which is not a history entry, since
        // entering a row pushes none, the same way opening Cloud Storage's
        // object browser does not. The flag says a drill is open; which level
        // it ascends to is `leave_bigquery_drilldown`'s answer, one per press.
        KeyCode::Esc
            if context.workspace == Workspace::Resources && context.resources_drilldown_open =>
        {
            Some(Action::Resources(ResourcesAction::LeaveDrillDown))
        }
        KeyCode::Esc if context.workspace == Workspace::Resources => {
            Some(Action::Navigation(NavAction::Back))
        }
        // `l` opens the Logging workspace. It is not a rebindable action, so it
        // stays a hardcoded arm (resolved after the override table above).
        KeyCode::Char('l') if is_plain_text_key(key) => {
            Some(Action::Workspace(WorkspaceAction::Logging))
        }
        // `Ctrl-t` opens the Tunnels workspace, with the same reach as `l`. The
        // CONTROL guard is load-bearing: a bare `t` is the Logging tail toggle,
        // and this arm sits ahead of the per-workspace dispatch below, so
        // widening it would shadow that key. Like `l` and `Y` it is not
        // rebindable, so it resolves after the override table above. The match
        // is exact rather than `.contains` for the same reason it is guarded at
        // all: `.contains` would hand `Ctrl-Alt-T` and `Ctrl-Shift-T` the
        // Tunnels workspace as well, chords the catalog never claims.
        KeyCode::Char('t') if key.modifiers == KeyModifiers::CONTROL => {
            Some(Action::Workspace(WorkspaceAction::Tunnels))
        }
        // `Ctrl-f` opens the FinOps workspace. The CONTROL guard is
        // load-bearing for the same reason `Ctrl-t`'s is: a bare `f` is the
        // resource filter, and this arm sits ahead of the per-workspace
        // dispatch below. Exact rather than `.contains`, so `Ctrl-Alt-F` and
        // `Ctrl-Shift-F` are not quietly claimed too.
        KeyCode::Char('f') if key.modifiers == KeyModifiers::CONTROL => {
            Some(Action::Workspace(WorkspaceAction::FinOps))
        }
        // Global copy-diagnostic (issue #31 AC #6): copies the last recorded
        // gcloud diagnostic's sanitized technical detail to the clipboard,
        // available in every normal-mode view, same reach as `r`/`?`. It is not
        // part of #36's rebindable set, so it stays a hardcoded arm (resolved
        // after the override table above), like `l` for Logging.
        KeyCode::Char('Y') if is_plain_text_key(key) => {
            Some(Action::System(SystemAction::CopyDiagnostic))
        }
        _ => match context.workspace {
            Workspace::Resources if context.resources_detail_open => {
                resource_detail_key(key, context)
            }
            Workspace::Resources => resource_list_key(key, context),
            Workspace::Configurations => configuration_list_key(key),
            Workspace::Logging => logging_key(key, context),
            Workspace::Tunnels => tunnels_key(key),
            Workspace::FinOps => finops_key(key),
            Workspace::Home => home_key(key),
        },
    }
}

/// The FinOps workspace has a single read-only view, so `Esc` is the only key
/// it owns.
///
/// No `:` arm, unlike its neighbours: the command palette is a *rebindable*
/// action and resolves through the override table in `normal_key` above,
/// before the per-workspace dispatch ever runs. The sibling arms in
/// `tunnels_key`, `logging_key` and `configuration_list_key` are therefore
/// unreachable too — pre-existing, and not widened here. `r` reaches the
/// FinOps refresh the same way, through `RebindableAction::Refresh`.
fn finops_key(key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Esc => Some(Action::Navigation(NavAction::Back)),
        _ => None,
    }
}

/// The Tunnels workspace has a single list view: move the selection, kill the
/// selected tunnel (`K`, deliberately shifted so a stray `k` can never signal a
/// child), dismiss a finished row, or leave via navigation history.
fn tunnels_key(key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Char(':') if is_plain_text_key(key) => Some(Action::Command(CommandAction::Open)),
        KeyCode::Char('j') if is_plain_text_key(key) => {
            Some(Action::Tunnels(TunnelsAction::SelectNext))
        }
        KeyCode::Down => Some(Action::Tunnels(TunnelsAction::SelectNext)),
        KeyCode::Char('k') if is_plain_text_key(key) => {
            Some(Action::Tunnels(TunnelsAction::SelectPrevious))
        }
        KeyCode::Up => Some(Action::Tunnels(TunnelsAction::SelectPrevious)),
        KeyCode::Char('K') if is_plain_text_key(key) => {
            Some(Action::Tunnels(TunnelsAction::KillSelected))
        }
        KeyCode::Char('d') if is_plain_text_key(key) => {
            Some(Action::Tunnels(TunnelsAction::DismissSelected))
        }
        KeyCode::Esc => Some(Action::Navigation(NavAction::Back)),
        _ => None,
    }
}

fn logging_key(key: KeyEvent, context: &Context) -> Option<Action> {
    if context.logging_detail_open {
        return logging_detail_key(key);
    }
    logging_list_key(key)
}

fn logging_detail_key(key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Char(':') if is_plain_text_key(key) => Some(Action::Command(CommandAction::Open)),
        KeyCode::Esc | KeyCode::Enter => Some(Action::Logging(LoggingAction::CloseDetail)),
        KeyCode::PageDown => Some(Action::Logging(LoggingAction::ScrollDetailDown)),
        KeyCode::PageUp => Some(Action::Logging(LoggingAction::ScrollDetailUp)),
        KeyCode::Char('w') if is_plain_text_key(key) => {
            Some(Action::Logging(LoggingAction::ToggleWrap))
        }
        KeyCode::Char('y') if is_plain_text_key(key) => {
            Some(Action::Logging(LoggingAction::CopySelectedEntry))
        }
        KeyCode::Char('i') if is_plain_text_key(key) => {
            Some(Action::Logging(LoggingAction::CopySelectedInsertId))
        }
        _ => None,
    }
}

fn logging_list_key(key: KeyEvent) -> Option<Action> {
    // `Ctrl-S` is resolved before every plain-character arm below: those arms
    // match on `key.code` alone, so an unguarded `Ctrl-S` would otherwise be
    // read as the `s` severity-floor key instead of the save key. The match is
    // exact rather than `.contains` for the same reason it is guarded at all:
    // `.contains` would also save on `Ctrl-Alt-S` and `Ctrl-Shift-S`, chords the
    // catalog never claims.
    if key.code == KeyCode::Char('s') && key.modifiers == KeyModifiers::CONTROL {
        return Some(Action::Logging(LoggingAction::SaveToFile));
    }

    match key.code {
        KeyCode::Char(':') if is_plain_text_key(key) => Some(Action::Command(CommandAction::Open)),
        KeyCode::Char('j') if is_plain_text_key(key) => {
            Some(Action::Logging(LoggingAction::SelectNext))
        }
        KeyCode::Down => Some(Action::Logging(LoggingAction::SelectNext)),
        KeyCode::Char('k') if is_plain_text_key(key) => {
            Some(Action::Logging(LoggingAction::SelectPrevious))
        }
        KeyCode::Up => Some(Action::Logging(LoggingAction::SelectPrevious)),
        KeyCode::Enter => Some(Action::Logging(LoggingAction::OpenDetail)),
        KeyCode::Esc => Some(Action::Navigation(NavAction::Back)),
        KeyCode::Char('t') if is_plain_text_key(key) => {
            Some(Action::Logging(LoggingAction::ToggleTail))
        }
        KeyCode::Char('T') if is_plain_text_key(key) => {
            Some(Action::Logging(LoggingAction::CycleTimeRange))
        }
        KeyCode::Char('s') if is_plain_text_key(key) => {
            Some(Action::Logging(LoggingAction::RaiseSeverityFloor))
        }
        KeyCode::Char('S') if is_plain_text_key(key) => {
            Some(Action::Logging(LoggingAction::LowerSeverityFloor))
        }
        KeyCode::Char('w') if is_plain_text_key(key) => {
            Some(Action::Logging(LoggingAction::ToggleWrap))
        }
        KeyCode::Char('/') if is_plain_text_key(key) => {
            Some(Action::Logging(LoggingAction::StartFilter))
        }
        KeyCode::Char('y') if is_plain_text_key(key) => {
            Some(Action::Logging(LoggingAction::CopySelectedEntry))
        }
        KeyCode::Char('i') if is_plain_text_key(key) => {
            Some(Action::Logging(LoggingAction::CopySelectedInsertId))
        }
        // Direct since-bucket addressing. Digits outside the seven buckets
        // resolve to `None` rather than to some nearest bucket, so an
        // unassigned digit stays unbound.
        KeyCode::Char(digit) if digit.is_ascii_digit() && is_plain_text_key(key) => {
            LogTimeRange::from_bucket_digit(digit)
                .map(|range| Action::Logging(LoggingAction::SetTimeRange(range)))
        }
        _ => None,
    }
}

fn logging_filter_key(key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Char(value) if is_plain_text_key(key) => {
            Some(Action::Logging(LoggingAction::AppendFilterChar(value)))
        }
        KeyCode::Backspace => Some(Action::Logging(LoggingAction::BackspaceFilter)),
        KeyCode::Enter => Some(Action::Logging(LoggingAction::ApplyFilter)),
        KeyCode::Esc => Some(Action::Logging(LoggingAction::CancelFilter)),
        _ => None,
    }
}

/// Converts a `KeyEvent` into the [`KeyTrigger`] used to look up a rebindable
/// action in the effective override table. Only single-character keys
/// (optionally with `Ctrl`) can be rebound, so all other keys yield `None`.
///
/// The `CONTROL` match is exact rather than `.contains`, for the same reason
/// the app's own `Ctrl-C`/`Ctrl-S`/`Ctrl-T` arms are: `.contains` would hand
/// `Ctrl-Alt-<x>` and `Ctrl-Shift-<x>` the user's `ctrl-<x>` binding too. Here
/// that matters more than anywhere else, because this is the one branch every
/// user-configured Ctrl hotkey passes through, so a loose guard widens the
/// operator's whole config at once — and the catalog side
/// (`trigger_matches_event(CtrlChar)`) requires exactly `CONTROL`, so a loose
/// guard also puts keymap and help catalog into open disagreement.
fn override_trigger(key: KeyEvent) -> Option<KeyTrigger> {
    match key.code {
        KeyCode::Char(c) if key.modifiers == KeyModifiers::CONTROL => Some(KeyTrigger::CtrlChar(c)),
        KeyCode::Char(c) if is_plain_text_key(key) => Some(KeyTrigger::Char(c)),
        _ => None,
    }
}

fn home_key(_key: KeyEvent) -> Option<Action> {
    // Home renders the read-only overview dashboard (issue #27); the service
    // navigator now lives in the Resources sidebar (issue #21/#42), not on Home.
    // Home therefore must NOT dispatch NavigatorActions — doing so would drive an
    // invisible navigator. Workspace navigation, refresh, help, quit AND the
    // command palette (`:`) are all rebindable and resolved via the effective
    // override table in `normal_key`, so Home has no non-rebindable keys of its
    // own.
    None
}

fn resource_list_key(key: KeyEvent, context: &Context) -> Option<Action> {
    match key.code {
        KeyCode::Char('v') if is_plain_text_key(key) => {
            Some(Action::Navigator(NavigatorAction::ToggleSidebar))
        }
        KeyCode::Tab => Some(Action::Navigator(NavigatorAction::ToggleFocus)),
        _ if context.service_sidebar_visible && context.service_navigator_focused => {
            resource_navigator_key(key)
        }
        _ => resource_content_key(key, context),
    }
}

fn resource_navigator_key(key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Char('/') if is_plain_text_key(key) => {
            Some(Action::Navigator(NavigatorAction::StartFilter))
        }
        KeyCode::Char('j') if is_plain_text_key(key) => {
            Some(Action::Navigator(NavigatorAction::SelectNext))
        }
        KeyCode::Down => Some(Action::Navigator(NavigatorAction::SelectNext)),
        KeyCode::Char('k') if is_plain_text_key(key) => {
            Some(Action::Navigator(NavigatorAction::SelectPrevious))
        }
        KeyCode::Up => Some(Action::Navigator(NavigatorAction::SelectPrevious)),
        KeyCode::Enter => Some(Action::Navigator(NavigatorAction::OpenSelected)),
        _ => None,
    }
}

/// The keys of the list pane itself.
///
/// Two of them are refused inside a resource drill-down, which is the one
/// place in this function where the *list on screen* and the *collection the
/// action reaches* are not the same thing. BigQuery's tables view is that
/// place today: `s` and `←`/`→` sort and walk the columns of the datasets
/// collection behind the drill, which no operator inside the drill can see
/// move. Both were being taught by the help overlay and the frame footer, and
/// a hint is a promise about the key it names — so rather than hide two keys
/// that still did something invisible, the keys stop doing it and leave both
/// catalogues honestly.
///
/// `Enter` used to be the third. It was refused because a table row opened
/// nothing; a table row now opens that table's column schema, so it
/// dispatches inside the drill like everywhere else and both catalogues teach
/// it again.
///
/// This is also what keeps `keymap_help_parity_test` meaningful over the
/// drill-down: that sweep pairs *dispatch* with catalogue metadata, so a key
/// dropped from the catalogue while still dispatching would fail it, and a key
/// dispatching while dropped from the catalogue is exactly the state it exists
/// to refuse.
fn resource_content_key(key: KeyEvent, context: &Context) -> Option<Action> {
    let drilldown = context.resources_drilldown_open;
    // The bottom of a drill-down: a level is open and nothing is under it.
    // False whenever no drill-down is open at all, which is what makes it safe
    // to read on its own in the `Enter` arm below.
    let leaf_drilldown = drilldown && !context.resources_drilldown_opens_another_level;
    match key.code {
        KeyCode::Char('/') if is_plain_text_key(key) => {
            Some(Action::Resources(ResourcesAction::StartFilter))
        }
        KeyCode::Char('s') if is_plain_text_key(key) && !drilldown => {
            Some(Action::Resources(ResourcesAction::SortBySelectedColumn))
        }
        KeyCode::Char('w') if is_plain_text_key(key) => {
            Some(Action::Resources(ResourcesAction::ToggleWideColumns))
        }
        // `is_plain_text_key`, not `is_empty`, like every other character arm
        // here. Both reject `Ctrl-Space`, which is what the guard is for; the
        // difference is `Shift-Space`, which the catalog's "Space" entry claims
        // and which this arm would otherwise refuse. Terminals only report
        // SHIFT alongside a character under the enhanced keyboard protocols,
        // so the refusal is latent rather than visible today — which is exactly
        // the kind of half-dead arm that goes unnoticed until the day it does
        // not.
        KeyCode::Char(' ') if is_plain_text_key(key) => {
            Some(Action::Resources(ResourcesAction::ToggleMark))
        }
        KeyCode::Char('V') if is_plain_text_key(key) => {
            Some(Action::Resources(ResourcesAction::RangeMark))
        }
        KeyCode::Char('\\') if is_plain_text_key(key) => {
            Some(Action::Resources(ResourcesAction::ClearMarks))
        }
        KeyCode::Char('U') if is_plain_text_key(key) => {
            Some(Action::Resources(ResourcesAction::UnmarkVisible))
        }
        // The empty-modifier guard is load-bearing: `Alt-Left`/`Alt-Right` are
        // the history keys, resolved earlier in `normal_key`, so a chord must
        // not silently reach the column cursor. Anything modified falls through
        // to `_ => None` and is simply unbound.
        KeyCode::Left if key.modifiers.is_empty() && !drilldown => {
            Some(Action::Resources(ResourcesAction::SelectPreviousColumn))
        }
        KeyCode::Right if key.modifiers.is_empty() && !drilldown => {
            Some(Action::Resources(ResourcesAction::SelectNextColumn))
        }
        KeyCode::Char('j') if is_plain_text_key(key) => {
            Some(Action::Resources(ResourcesAction::SelectNext))
        }
        KeyCode::Down => Some(Action::Resources(ResourcesAction::SelectNext)),
        KeyCode::Char('k') if is_plain_text_key(key) => {
            Some(Action::Resources(ResourcesAction::SelectPrevious))
        }
        KeyCode::Up => Some(Action::Resources(ResourcesAction::SelectPrevious)),
        // Unguarded on modifiers, like every other `Enter` arm — see
        // `control_keys_ignore_modifiers`.
        //
        // Guarded on the drill-down's *depth*, which it was not always.
        // Inside BigQuery's tables view `Enter` opens the selected table's
        // column schema; inside the schema view a column is the bottom of the
        // hierarchy and there is nothing under it. That used to be left to the
        // reducer, which no-ops on a level with nothing below it — and the
        // no-op is still there, so this changes nothing an operator can see.
        //
        // What it changes is that the catalogue may now stop teaching `Enter`
        // at the bottom level. `keymap_help_parity_test` requires every key
        // this function dispatches in a context to carry catalogue metadata
        // for that context, so a catalogue that dropped the binding while this
        // arm still answered would be refused by the gate that keeps the two
        // in step — which is why the overlay taught a dead key for three
        // slices rather than being edited alone.
        KeyCode::Enter if !leaf_drilldown => Some(Action::Resources(ResourcesAction::OpenSelected)),
        // Cloud Storage-specific actions (issue #34); the reducer no-ops
        // these for non-storage services, so it's safe to dispatch them
        // unconditionally from the generic resource list.
        KeyCode::Char('i') if is_plain_text_key(key) => {
            Some(Action::Resources(ResourcesAction::OpenBucketMetadata))
        }
        // `t` starts a managed tunnel for the selected row. Dispatched for
        // every service for the same reason the storage keys above are: the
        // reducer no-ops it where there is no tunnel plan (GKE, Cloud Run,
        // Cloud Storage). A bare `t` is free here — the Tunnels *workspace* is
        // `Ctrl-t`, and the Logging tail toggle is a different workspace.
        KeyCode::Char('t') if is_plain_text_key(key) => {
            Some(Action::Resources(ResourcesAction::RequestTunnel))
        }
        KeyCode::Char('y') if is_plain_text_key(key) => {
            Some(Action::Resources(ResourcesAction::CopyResourceUri))
        }
        KeyCode::Char('n') if is_plain_text_key(key) => {
            Some(Action::Resources(ResourcesAction::NextPage))
        }
        // `L` (shifted, so it cannot be confused with the `l` that opens the
        // Logging workspace) opens logs scoped to the current selection. The
        // plain-key guard keeps `Ctrl-L` free, matching the arms above it.
        KeyCode::Char('L') if is_plain_text_key(key) => {
            Some(Action::Resources(ResourcesAction::OpenLogsForSelection))
        }
        // `J` (shifted, next to `L`) offers the related-resource jumps for the
        // current selection. It is bound here rather than gated inside
        // `applicable_jumps` so it stays unbound outside the resource views,
        // where there is no selection for it to be about.
        KeyCode::Char('J') if is_plain_text_key(key) => Some(Action::Jumps(JumpsAction::Open)),
        // `,` arms the plugin prefix. Dispatched for every service
        // rather than gated here, like the `t` and `y` arms above: the reducer
        // holds the plugin set, so it is where "this service has no plugin"
        // can be both decided and explained.
        KeyCode::Char(',') if is_plain_text_key(key) => {
            Some(Action::Resources(ResourcesAction::ArmPluginPrefix))
        }
        KeyCode::Char('-') if is_plain_text_key(key) => {
            Some(Action::Resources(ResourcesAction::AscendPrefix))
        }
        KeyCode::Backspace => Some(Action::Resources(ResourcesAction::AscendPrefix)),
        _ => None,
    }
}

fn resource_detail_key(key: KeyEvent, context: &Context) -> Option<Action> {
    match key.code {
        KeyCode::Char('v') if is_plain_text_key(key) => {
            Some(Action::Navigator(NavigatorAction::ToggleSidebar))
        }
        KeyCode::Tab => Some(Action::Navigator(NavigatorAction::ToggleFocus)),
        _ if context.service_sidebar_visible && context.service_navigator_focused => {
            resource_navigator_key(key)
        }
        KeyCode::Char('j') if is_plain_text_key(key) => {
            Some(Action::Resources(ResourcesAction::SelectNext))
        }
        KeyCode::Down => Some(Action::Resources(ResourcesAction::SelectNext)),
        KeyCode::Char('k') if is_plain_text_key(key) => {
            Some(Action::Resources(ResourcesAction::SelectPrevious))
        }
        KeyCode::Up => Some(Action::Resources(ResourcesAction::SelectPrevious)),
        KeyCode::PageDown => Some(Action::Resources(ResourcesAction::ScrollDetailDown)),
        KeyCode::PageUp => Some(Action::Resources(ResourcesAction::ScrollDetailUp)),
        KeyCode::Char(']') if is_plain_text_key(key) => {
            Some(Action::Resources(ResourcesAction::SelectNextTab))
        }
        KeyCode::Char('[') if is_plain_text_key(key) => {
            Some(Action::Resources(ResourcesAction::SelectPreviousTab))
        }
        // Field cursor over the pane's copyable values; `y`
        // below yanks whichever field it lands on.
        KeyCode::Char('n') if is_plain_text_key(key) => {
            Some(Action::Resources(ResourcesAction::SelectNextDetailField))
        }
        KeyCode::Char('p') if is_plain_text_key(key) => Some(Action::Resources(
            ResourcesAction::SelectPreviousDetailField,
        )),
        KeyCode::Enter => Some(Action::Resources(ResourcesAction::OpenSelected)),
        // Copying the current bucket/prefix/object URI is also useful from a
        // detail pane (issue #34); the reducer resolves the URI from the
        // storage state's current selection regardless of view.
        KeyCode::Char('y') if is_plain_text_key(key) => {
            Some(Action::Resources(ResourcesAction::CopyResourceUri))
        }
        // Same key as in the list. A detail pane has a resource on screen too,
        // and the reducer scopes the jump to that pane's target rather than to
        // the list cursor behind it.
        KeyCode::Char('L') if is_plain_text_key(key) => {
            Some(Action::Resources(ResourcesAction::OpenLogsForSelection))
        }
        // Same key as in the list, same reason: `applicable_jumps` already
        // scopes the offers to this pane's target rather than to the list
        // cursor behind it.
        KeyCode::Char('J') if is_plain_text_key(key) => Some(Action::Jumps(JumpsAction::Open)),
        // Same key as in the list, same reason: a detail pane has a resource on
        // screen too, and the reducer resolves the plugin's template against
        // that pane's target rather than the list cursor behind it.
        KeyCode::Char(',') if is_plain_text_key(key) => {
            Some(Action::Resources(ResourcesAction::ArmPluginPrefix))
        }
        _ => None,
    }
}

fn resource_filter_key(key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Char(value) if is_plain_text_key(key) => {
            Some(Action::Resources(ResourcesAction::AppendFilterChar(value)))
        }
        KeyCode::Backspace => Some(Action::Resources(ResourcesAction::BackspaceFilter)),
        KeyCode::Enter => Some(Action::Resources(ResourcesAction::ApplyFilter)),
        KeyCode::Esc => Some(Action::Resources(ResourcesAction::CancelFilter)),
        _ => None,
    }
}

fn service_filter_key(key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Char(value) if is_plain_text_key(key) => {
            Some(Action::Navigator(NavigatorAction::AppendFilterChar(value)))
        }
        KeyCode::Backspace => Some(Action::Navigator(NavigatorAction::BackspaceFilter)),
        KeyCode::Enter => Some(Action::Navigator(NavigatorAction::ApplyFilter)),
        KeyCode::Esc => Some(Action::Navigator(NavigatorAction::CancelFilter)),
        _ => None,
    }
}

fn command_palette_key(key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Down => Some(Action::Command(CommandAction::SelectNext)),
        KeyCode::Up => Some(Action::Command(CommandAction::SelectPrevious)),
        KeyCode::Char(value) if is_plain_text_key(key) => {
            Some(Action::Command(CommandAction::AppendChar(value)))
        }
        KeyCode::Backspace => Some(Action::Command(CommandAction::Backspace)),
        KeyCode::Enter => Some(Action::Command(CommandAction::Submit)),
        KeyCode::Esc => Some(Action::Command(CommandAction::Cancel)),
        _ => None,
    }
}

fn configuration_list_key(key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Char('/') if is_plain_text_key(key) => {
            Some(Action::Configurations(ConfigurationsAction::StartFiltering))
        }
        KeyCode::Char('u') if is_plain_text_key(key) => {
            Some(Action::Configurations(ConfigurationsAction::ClearFilter))
        }
        KeyCode::Char('a') if is_plain_text_key(key) => {
            Some(Action::Configurations(ConfigurationsAction::StartCreate))
        }
        KeyCode::Char('e') if is_plain_text_key(key) => Some(Action::Configurations(
            ConfigurationsAction::StartEditActiveContext,
        )),
        KeyCode::Char('j') if is_plain_text_key(key) => {
            Some(Action::Configurations(ConfigurationsAction::SelectNext))
        }
        KeyCode::Down => Some(Action::Configurations(ConfigurationsAction::SelectNext)),
        KeyCode::Char('k') if is_plain_text_key(key) => {
            Some(Action::Configurations(ConfigurationsAction::SelectPrevious))
        }
        KeyCode::Up => Some(Action::Configurations(ConfigurationsAction::SelectPrevious)),
        KeyCode::Enter => Some(Action::Configurations(
            ConfigurationsAction::RequestActivateSelected,
        )),
        _ => None,
    }
}

fn configuration_filter_key(key: KeyEvent, context: &Context) -> Option<Action> {
    match key.code {
        KeyCode::Char(value) if is_plain_text_key(key) => {
            let mut next = context.configurations_filter_draft.clone();
            next.push(value);
            Some(Action::Configurations(ConfigurationsAction::UpdateFilter(
                next,
            )))
        }
        KeyCode::Backspace => {
            let mut next = context.configurations_filter_draft.clone();
            next.pop();
            Some(Action::Configurations(ConfigurationsAction::UpdateFilter(
                next,
            )))
        }
        KeyCode::Enter => Some(Action::Configurations(ConfigurationsAction::ApplyFilter)),
        KeyCode::Esc => Some(Action::Configurations(ConfigurationsAction::CancelFilter)),
        _ => None,
    }
}

fn configuration_create_key(key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Char(value) if is_plain_text_key(key) => Some(Action::Configurations(
            ConfigurationsAction::AppendCreateChar(value),
        )),
        KeyCode::Backspace => Some(Action::Configurations(
            ConfigurationsAction::BackspaceCreateField,
        )),
        KeyCode::Tab | KeyCode::Down => Some(Action::Configurations(
            ConfigurationsAction::FocusNextCreateField,
        )),
        KeyCode::BackTab | KeyCode::Up => Some(Action::Configurations(
            ConfigurationsAction::FocusPreviousCreateField,
        )),
        KeyCode::Enter => Some(Action::Configurations(ConfigurationsAction::SubmitCreate)),
        KeyCode::Esc => Some(Action::Configurations(ConfigurationsAction::CancelCreate)),
        _ => None,
    }
}

fn configuration_context_edit_key(key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Char(' ') if key.modifiers == KeyModifiers::CONTROL => Some(
            Action::Configurations(ConfigurationsAction::OpenContextPicker),
        ),
        KeyCode::Char(value) if is_plain_text_key(key) => Some(Action::Configurations(
            ConfigurationsAction::AppendContextEditChar(value),
        )),
        KeyCode::Backspace => Some(Action::Configurations(
            ConfigurationsAction::BackspaceContextEditField,
        )),
        KeyCode::Tab | KeyCode::Down => Some(Action::Configurations(
            ConfigurationsAction::FocusNextContextEditField,
        )),
        KeyCode::BackTab | KeyCode::Up => Some(Action::Configurations(
            ConfigurationsAction::FocusPreviousContextEditField,
        )),
        KeyCode::Enter => Some(Action::Configurations(
            ConfigurationsAction::SubmitContextEdit,
        )),
        KeyCode::Esc => Some(Action::Configurations(
            ConfigurationsAction::CancelContextEdit,
        )),
        _ => None,
    }
}

fn configuration_context_picker_key(key: KeyEvent, context: &Context) -> Option<Action> {
    if context.configuration_context_picker_filter_open {
        return configuration_context_picker_filter_key(key, context);
    }

    match key.code {
        KeyCode::Char('/') if is_plain_text_key(key) => Some(Action::Configurations(
            ConfigurationsAction::StartContextPickerFilter,
        )),
        KeyCode::Char('j') if is_plain_text_key(key) => Some(Action::Configurations(
            ConfigurationsAction::SelectNextContextPickerItem,
        )),
        KeyCode::Down => Some(Action::Configurations(
            ConfigurationsAction::SelectNextContextPickerItem,
        )),
        KeyCode::Char('k') if is_plain_text_key(key) => Some(Action::Configurations(
            ConfigurationsAction::SelectPreviousContextPickerItem,
        )),
        KeyCode::Up => Some(Action::Configurations(
            ConfigurationsAction::SelectPreviousContextPickerItem,
        )),
        KeyCode::Char('r') if is_plain_text_key(key) => Some(Action::Configurations(
            ConfigurationsAction::RetryContextPicker,
        )),
        KeyCode::Enter => Some(Action::Configurations(
            ConfigurationsAction::ApplyContextPickerSelection,
        )),
        KeyCode::Esc => Some(Action::Configurations(
            ConfigurationsAction::CancelContextPicker,
        )),
        _ => None,
    }
}

fn configuration_context_picker_filter_key(key: KeyEvent, context: &Context) -> Option<Action> {
    match key.code {
        KeyCode::Char(value) if is_plain_text_key(key) => {
            let mut next = context.configuration_context_picker_filter_draft.clone();
            next.push(value);
            Some(Action::Configurations(
                ConfigurationsAction::UpdateContextPickerFilter(next),
            ))
        }
        KeyCode::Backspace => {
            let mut next = context.configuration_context_picker_filter_draft.clone();
            next.pop();
            Some(Action::Configurations(
                ConfigurationsAction::UpdateContextPickerFilter(next),
            ))
        }
        KeyCode::Enter => Some(Action::Configurations(
            ConfigurationsAction::ApplyContextPickerFilter,
        )),
        KeyCode::Esc => Some(Action::Configurations(
            ConfigurationsAction::CancelContextPickerFilter,
        )),
        _ => None,
    }
}

fn is_plain_text_key(key: KeyEvent) -> bool {
    key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT
}

fn help_key(key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Char('j') if is_plain_text_key(key) => {
            Some(Action::System(SystemAction::ScrollHelpDown))
        }
        KeyCode::Down => Some(Action::System(SystemAction::ScrollHelpDown)),
        KeyCode::Char('k') if is_plain_text_key(key) => {
            Some(Action::System(SystemAction::ScrollHelpUp))
        }
        KeyCode::Up => Some(Action::System(SystemAction::ScrollHelpUp)),
        KeyCode::PageDown => Some(Action::System(SystemAction::ScrollHelpPageDown)),
        KeyCode::PageUp => Some(Action::System(SystemAction::ScrollHelpPageUp)),
        KeyCode::Char('g') if is_plain_text_key(key) => {
            Some(Action::System(SystemAction::ScrollHelpTop))
        }
        KeyCode::Char('G') if is_plain_text_key(key) => {
            Some(Action::System(SystemAction::ScrollHelpBottom))
        }
        KeyCode::Esc => Some(Action::System(SystemAction::ClearOverlay)),
        KeyCode::Char('?') if is_plain_text_key(key) => {
            Some(Action::System(SystemAction::ClearOverlay))
        }
        KeyCode::Char('q') if is_plain_text_key(key) => Some(Action::QuitRequested),
        _ => None,
    }
}

fn confirm_configuration_key(key: KeyEvent, context: &Context) -> Option<Action> {
    match key.code {
        KeyCode::Enter if context.configuration_activation_confirmation_preview_visible => Some(
            Action::Configurations(ConfigurationsAction::ConfirmActivation),
        ),
        KeyCode::Char('y')
            if is_plain_text_key(key)
                && context.configuration_activation_confirmation_preview_visible =>
        {
            Some(Action::Configurations(
                ConfigurationsAction::ConfirmActivation,
            ))
        }
        KeyCode::Esc => Some(Action::Configurations(
            ConfigurationsAction::CancelActivation,
        )),
        KeyCode::Char('n') if is_plain_text_key(key) => Some(Action::Configurations(
            ConfigurationsAction::CancelActivation,
        )),
        _ => None,
    }
}

fn resource_action_confirmation_key(key: KeyEvent, context: &Context) -> Option<Action> {
    if context.resource_batch_confirmation_open {
        return resource_batch_confirmation_key(key, context);
    }
    match key.code {
        KeyCode::Enter if context.resource_action_confirmation_preview_visible => {
            Some(Action::Resources(ResourcesAction::ConfirmPendingAction))
        }
        KeyCode::Esc => Some(Action::Resources(ResourcesAction::CancelPendingAction)),
        _ => None,
    }
}

fn resource_batch_confirmation_key(key: KeyEvent, context: &Context) -> Option<Action> {
    match key.code {
        KeyCode::Enter
            if context.resource_batch_ack_satisfied
                && context.resource_batch_confirmation_visible =>
        {
            Some(Action::Resources(ResourcesAction::ConfirmBatchAction))
        }
        KeyCode::Char(value) if value.is_ascii_digit() && is_plain_text_key(key) => Some(
            Action::Resources(ResourcesAction::BatchAckAppendDigit(value)),
        ),
        KeyCode::Backspace => Some(Action::Resources(ResourcesAction::BatchAckBackspace)),
        KeyCode::PageDown => Some(Action::Resources(ResourcesAction::ScrollBatchConfirmDown)),
        KeyCode::PageUp => Some(Action::Resources(ResourcesAction::ScrollBatchConfirmUp)),
        KeyCode::Esc => Some(Action::Resources(ResourcesAction::CancelBatchAction)),
        _ => None,
    }
}

fn resource_command_preview_key(key: KeyEvent, context: &Context) -> Option<Action> {
    match key.code {
        KeyCode::Enter if context.resource_command_preview_visible => {
            Some(Action::Resources(ResourcesAction::CopyPendingCommand))
        }
        KeyCode::Esc => Some(Action::Resources(
            ResourcesAction::CancelPendingCommandPreview,
        )),
        _ => None,
    }
}

/// The typed-input prompt.
///
/// No `plugin_prompt_open` conjunct on any arm: the dispatch arm that routes
/// here already requires it, and repeating it makes both guards
/// non-load-bearing — the mutant that deletes either survives, which is what
/// was recorded on `Mode::PluginConfirmation`. The division kept is the
/// sibling modals': the outer arm guards the modal's existence, these guard its
/// actionable state.
///
/// `Enter` is gated on both flags for the reason the confirmation's is, one
/// step earlier: it composes the argv the operator is about to approve, out of
/// values they can only check on a modal the terminal can draw. The editing
/// arms carry neither gate — typing is the only way out of an unsatisfied
/// prompt, and `Esc` has to close one that cannot be read.
fn plugin_prompt_key(key: KeyEvent, context: &Context) -> Option<Action> {
    match key.code {
        KeyCode::Enter if context.plugin_prompt_satisfied && context.plugin_prompt_visible => {
            Some(Action::Resources(ResourcesAction::SubmitPluginPrompt))
        }
        KeyCode::Tab => Some(Action::Resources(ResourcesAction::PluginPromptNextField)),
        KeyCode::BackTab => Some(Action::Resources(
            ResourcesAction::PluginPromptPreviousField,
        )),
        // The empty-modifier guard is the one the column cursor carries, for
        // the same reason: `Alt-Left`/`Alt-Right` are the history keys, and a
        // chord must not reach a dropdown through a modal that never claimed
        // it. It is also the only form the help catalog can express — a
        // `KeyTrigger::Left` means the bare arrow — so an unguarded arm would
        // dispatch a key the footer cannot name.
        KeyCode::Right if key.modifiers.is_empty() => Some(Action::Resources(
            ResourcesAction::PluginPromptCycleChoice(true),
        )),
        KeyCode::Left if key.modifiers.is_empty() => Some(Action::Resources(
            ResourcesAction::PluginPromptCycleChoice(false),
        )),
        KeyCode::Backspace => Some(Action::Resources(ResourcesAction::PluginPromptBackspace)),
        KeyCode::Esc => Some(Action::Resources(ResourcesAction::CancelPluginPrompt)),
        // Last, so the arms above claim their keys first. `is_plain_text_key`
        // is what keeps every `Ctrl-<char>` chord out, `Ctrl-x` included —
        // though that one never reaches here, having resolved before the mode
        // match.
        KeyCode::Char(value) if is_plain_text_key(key) => Some(Action::Resources(
            ResourcesAction::PluginPromptAppend(value),
        )),
        _ => None,
    }
}

/// A plugin's confirmation. Its `Enter` is gated on the typed
/// acknowledgement a `dangerous` plugin demands — for an ordinary plugin the
/// flag is already true, so the keypress alone confirms — and on the modal
/// being readable, like `resource_command_preview_key` and
/// `ssh_launch_preview_key` above and below it. On a terminal too small for the
/// modal the operator sees "needs a larger terminal" and no argv at all, and
/// this `Enter` runs the command: acting on a confirmation nobody could read is
/// the one thing the gate exists to prevent.
///
/// The digit and `Backspace` arms are ungated on purpose: the acknowledgement
/// has to be editable in both directions, and a guard reading
/// `plugin_confirmation_ack_satisfied` would refuse `Backspace` at exactly the
/// moment a mistyped count became satisfiable. They dispatch for an ordinary
/// plugin too, where the reducer has nothing to append to — which is why the
/// catalog names them for this context rather than pretending only `Enter` and
/// `Esc` arrive here. They are ungated on the visibility flag as well: typing
/// into an ack nobody can read changes nothing on its own, and `Esc` must keep
/// closing a modal the terminal is too small to show.
fn plugin_confirmation_key(key: KeyEvent, context: &Context) -> Option<Action> {
    match key.code {
        // No `plugin_confirmation_open` conjunct here: the dispatch arm that
        // routes here already requires it, so repeating it made the two guards
        // mutually redundant — either one alone answers `None`, so neither is
        // load-bearing and a mutation of either survives. The division that
        // remains is the one the sibling modals use: the outer arm guards the
        // modal's *existence*, this guard its readable and actionable *state*.
        KeyCode::Enter
            if context.plugin_confirmation_ack_satisfied && context.plugin_confirmation_visible =>
        {
            Some(Action::Resources(ResourcesAction::ConfirmPlugin))
        }
        KeyCode::Char(value) if value.is_ascii_digit() && is_plain_text_key(key) => Some(
            Action::Resources(ResourcesAction::PluginAckAppendDigit(value)),
        ),
        KeyCode::Backspace => Some(Action::Resources(ResourcesAction::PluginAckBackspace)),
        KeyCode::Esc => Some(Action::Resources(ResourcesAction::CancelPluginConfirmation)),
        _ => None,
    }
}

/// The character after `,`.
///
/// No `plugin_prefix_pending` conjunct on the arm that reaches this, and none
/// here: `Mode::PluginPrefix` has a single assignment in the `input_context`
/// ladder and it is that flag, so a guard restating it could never be false
/// when the arm matched. That is the equivalent mutant the gate flagged on
/// `Mode::PluginConfirmation` — no test could kill it, and the only
/// way green was deleting the expression.
///
/// Every character dispatches, including ones no plugin declares: the reducer
/// holds the plugin set and is the only place that can tell "bound but could
/// not resolve for this row" from "never declared in this scope", which are
/// different sentences to a different reader.
///
/// A non-character key resolves to `None` and the prefix stays armed, rather
/// than falling through to the resource list's own bindings. An armed prefix
/// that let `Down` move the cursor would leave the operator's next character
/// resolving as a plugin key against a different row than the one they were
/// looking at when they pressed `,`.
fn plugin_prefix_key(key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Char(value) if is_plain_text_key(key) => {
            Some(Action::Resources(ResourcesAction::PluginPrefixPress(value)))
        }
        KeyCode::Esc => Some(Action::Resources(ResourcesAction::CancelPluginPrefix)),
        _ => None,
    }
}

fn ssh_launch_preview_key(key: KeyEvent, context: &Context) -> Option<Action> {
    match key.code {
        KeyCode::Enter if context.ssh_launch_preview_visible => {
            Some(Action::Resources(ResourcesAction::ConfirmSshLaunch))
        }
        KeyCode::Esc => Some(Action::Resources(ResourcesAction::CancelSshLaunch)),
        _ => None,
    }
}

/// The managed-tunnel launch confirmation. `Enter` is gated on the preview
/// actually being on screen, like every other launch modal: confirming an argv
/// the terminal is too small to show would be accepting something unread.
///
/// `Instant::now()` is read here, at the input boundary, because the reducer
/// must never read the clock and the tunnel entry it inserts needs a
/// `started_at`. This is the only impure call in the keymap; the instant only
/// ever flows into `TunnelEntry::started_at`, which the tunnel list ages
/// against.
fn tunnel_launch_preview_key(key: KeyEvent, context: &Context) -> Option<Action> {
    match key.code {
        KeyCode::Enter if context.tunnel_launch_preview_visible => {
            Some(Action::Resources(ResourcesAction::ConfirmTunnelLaunch {
                started_at: std::time::Instant::now(),
            }))
        }
        KeyCode::Esc => Some(Action::Resources(ResourcesAction::CancelTunnelLaunch)),
        _ => None,
    }
}

fn k9s_launch_confirmation_key(key: KeyEvent, context: &Context) -> Option<Action> {
    match key.code {
        KeyCode::Enter if context.k9s_launch_confirmation_preview_visible => {
            Some(Action::Resources(ResourcesAction::ConfirmK9sLaunch))
        }
        KeyCode::Esc => Some(Action::Resources(ResourcesAction::CancelK9sLaunch)),
        _ => None,
    }
}
