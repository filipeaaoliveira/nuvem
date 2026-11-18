//! Evidence generator — the three log-viewer follow-ups from the
//! whole-branch review.
//!
//! Each of the three sections below states the defect it forecloses and then
//! proves it is gone, through the same public seams the app itself uses. None
//! of the three defects announced itself: a stolen key, a spinner, and a label
//! are all things the operator reads as normal, so the only place they can be
//! caught is here.
//!
//! 1. **`Ctrl-S` was two keys at once.** In the resource list it fired
//!    `SortBySelectedColumn`, because that arm matched `KeyCode::Char('s')`
//!    without looking at the modifiers; in the Logging list the same chord saved
//!    the visible buffer. Whichever one the operator wanted, the other was one
//!    workspace away. The same modifier-blindness let every other
//!    `Ctrl-<letter>` fall into whatever plain-letter arm shared its character,
//!    so this section proves the *class* is closed and not just the instance:
//!    no `Ctrl-<letter>` dispatches anything the footer/help catalog does not
//!    also teach, in any `HelpContext`. Separately, `ctrl-s` and `ctrl-t` are
//!    now refused by `[hotkeys]` validation, with a reason that states the true
//!    mechanism — the override table resolves *before* the app's own keys, so
//!    such a binding wins and silently steals the app's key.
//!
//! 2. **A Logging deep link could strand the viewer at `Loading`.** Restoring
//!    the Logging location marked the buffer `Loading` and then asked for a
//!    fetch that, with no resolved context, was never dispatched — and the tail
//!    poll skips for the same reason, so nothing would ever clear the spinner.
//!    The positive counterpart is asserted alongside it, so "settles to
//!    `Initial`" cannot be satisfied by never showing a spinner at all.
//!
//! 3. **The log search filter compiled twice per frame.** `visible_entries`
//!    re-parsed the applied pattern on every call and the header's mode label
//!    compiled a second filter just to name the mode. Both are gone: the label
//!    is now derived from the pure-syntax view with no compile, and the compiled
//!    filter is cached beside its source. Those two changes create two *new*
//!    ways to be wrong — a label that describes a different pattern than the
//!    rows were matched against, and a cache that outlives the source it was
//!    built from — and this section is aimed at both. It contains no timing
//!    assertion of any kind: the saved work is a property of the code's shape,
//!    and what is worth pinning is that the shape did not cost correctness.
//!
//! Every assertion runs unconditionally. When `NUVEM_EVIDENCE_DIR` is set, each
//! section also writes a transcript of what it observed; with the variable
//! unset the test writes nothing at all and never writes outside that
//! directory.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

use nuvem::app::AppState;
use nuvem::app::projection::binding::{
    ALL_HELP_CONTEXTS, HelpContext, KeyTrigger, context_bindings,
};
use nuvem::app::projection::logging::project_logging;
use nuvem::app::reducer::root::reduce;
use nuvem::app::state::logging::LoggingStatus;
use nuvem::app::state::root::{RootState, Workspace};
use nuvem::app::{
    Action, Effect, EffectResult, LoggingAction, NavAction, ResourcesAction, WorkspaceAction,
};
use nuvem::config::hotkeys::KeyOverrides;
use nuvem::input::keymap::action_for_key;
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use nuvem_domain::gcp::logging::{LogEntry, LogPayload, LogSeverity};
use nuvem_domain::gcp::text_filter::{TextFilter, TextFilterKind, TextFilterSyntax};

use crate::support::keymap_context;

use keymap_context::context_for;

// ---------------------------------------------------------------------------
// Evidence writing
// ---------------------------------------------------------------------------

/// The evidence directory, or `None` when `NUVEM_EVIDENCE_DIR` is unset.
///
/// There is deliberately no fallback location: with the variable unset this
/// test writes nothing, so it cannot collide with a concurrent run or leave
/// files behind on a developer's machine.
fn evidence_dir() -> Option<PathBuf> {
    let dir = std::env::var("NUVEM_EVIDENCE_DIR").ok()?;
    let path = PathBuf::from(dir);
    fs::create_dir_all(&path).ok()?;
    Some(path)
}

/// Accumulates one section's transcript. Each section owns its own file, so the
/// three tests can run concurrently without sharing state.
#[derive(Default)]
struct Transcript {
    body: String,
}

impl Transcript {
    fn line(&mut self, text: &str) {
        self.body.push_str(text);
        self.body.push('\n');
    }

    fn section(&mut self, title: &str) {
        self.line("");
        self.line(title);
        self.line(&"-".repeat(title.len()));
    }

    fn write(self, name: &str) {
        let Some(dir) = evidence_dir() else {
            return;
        };
        fs::write(dir.join(name), self.body).unwrap_or_else(|e| panic!("write {name}: {e}"));
    }
}

// ---------------------------------------------------------------------------
// Key probing
// ---------------------------------------------------------------------------

fn press(code: KeyCode) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: KeyModifiers::empty(),
        kind: KeyEventKind::Press,
        state: KeyEventState::empty(),
    }
}

fn ctrl(ch: char) -> KeyEvent {
    KeyEvent {
        code: KeyCode::Char(ch),
        modifiers: KeyModifiers::CONTROL,
        kind: KeyEventKind::Press,
        state: KeyEventState::empty(),
    }
}

/// Builds the raw `[hotkeys]` table from `(action, key)` pairs.
fn raw(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

/// The one error line mentioning `key`, from a rejected `[hotkeys]` table.
fn rejection_for(key: &str) -> String {
    let errors = KeyOverrides::from_file(&raw(&[("home", key)]))
        .err()
        .unwrap_or_else(|| panic!("binding home to {key} must be rejected"));
    let matching: Vec<&String> = errors.iter().filter(|e| e.contains(key)).collect();
    assert_eq!(
        matching.len(),
        1,
        "expected exactly one error naming {key}: {errors:?}"
    );
    matching[0].clone()
}

// ---------------------------------------------------------------------------
// 1. Ctrl-S is one key again, and the whole Ctrl-<letter> class is closed
// ---------------------------------------------------------------------------

/// Forecloses: `Ctrl-S` firing `SortBySelectedColumn` in the resource list
/// (`resource_content_key`'s `s` arm matched on `key.code` alone), a
/// `[hotkeys]` entry on `ctrl-s`/`ctrl-t` silently stealing the app's key, and
/// the parity harness being unable to see either because `key_universe` swept
/// only three hand-picked Ctrl chords.
#[test]
fn ctrl_s_is_one_key_and_no_ctrl_letter_leaks_into_a_plain_letter_arm() {
    let mut t = Transcript::default();
    t.line(
        "Item 1 — Ctrl-S dispatched two different actions depending on the\n\
         workspace, and every other Ctrl-<letter> could fall into the plain-letter arm\n\
         sharing its character. Below: the named instance, the config-validation half,\n\
         and the whole class swept across every help context.",
    );

    // ---- 1a. The named bug: Ctrl-S in the resource list -------------------
    t.section("1a. The instance — Ctrl-S in the resource list");

    let resources = context_for(HelpContext::ResourceList);
    let logging = context_for(HelpContext::LoggingList);

    assert_eq!(
        action_for_key(ctrl('s'), &resources),
        None,
        "Ctrl-S must be unbound in the resource list — it used to sort by the selected \
         column, so the operator's save-the-logs muscle memory reordered a table instead"
    );
    // The control for the line above: the guard rejected the modifier, it did
    // not delete the binding. Plain `s` still sorts.
    assert_eq!(
        action_for_key(press(KeyCode::Char('s')), &resources),
        Some(Action::Resources(ResourcesAction::SortBySelectedColumn)),
        "plain `s` must still sort the resource list"
    );
    // ...and the key Ctrl-S was colliding *with* still works where it belongs.
    assert_eq!(
        action_for_key(ctrl('s'), &logging),
        Some(Action::Logging(LoggingAction::SaveToFile)),
        "Ctrl-S must still save the visible buffer in the Logging list"
    );
    assert_eq!(
        action_for_key(press(KeyCode::Char('s')), &logging),
        Some(Action::Logging(LoggingAction::RaiseSeverityFloor)),
        "plain `s` must still raise the severity floor in the Logging list"
    );
    t.line("  ResourceList  Ctrl-S -> (unbound)                 [was: SortBySelectedColumn]");
    t.line("  ResourceList  s      -> Resources(SortBySelectedColumn)");
    t.line("  LoggingList   Ctrl-S -> Logging(SaveToFile)");
    t.line("  LoggingList   s      -> Logging(RaiseSeverityFloor)");

    // ---- 1b. `[hotkeys]` refuses ctrl-s / ctrl-t, and says why ------------
    t.section("1b. The config half — ctrl-s and ctrl-t are reserved, with the true reason");

    // The reason matters as much as the rejection. A user whose config stops
    // loading is owed the fact that it was previously doing something other
    // than what they thought: their binding *won*, because the override table
    // is consulted before the app's own keys.
    for (key, stolen) in [("ctrl-s", "Logging save-to-file"), ("ctrl-t", "Tunnels")] {
        let error = rejection_for(key);
        assert!(
            error.contains("reserved key"),
            "{key}: the error must name the key as reserved: {error}"
        );
        assert!(
            error.contains("the override table is resolved before the app's own keys"),
            "{key}: the error must state the dispatch order that makes the binding \
             dangerous, not merely that it is refused: {error}"
        );
        assert!(
            error.contains("steal"),
            "{key}: the error must say the binding wins and steals the app's key — the \
             opposite of the inert-binding case Ctrl-C and Esc are reserved for: {error}"
        );
        assert!(
            error.contains(stolen),
            "{key}: the error must name the key that would be stolen ({stolen}): {error}"
        );
        t.line(&format!("  {error}"));
    }

    // The control: validation rejects these two *because they are reserved*,
    // not because it refuses Ctrl chords in general. Without this line the
    // block above would also pass against a validator that banned every
    // `ctrl-<letter>`.
    KeyOverrides::from_file(&raw(&[("home", "ctrl-g")]))
        .expect("an unreserved Ctrl chord must still be bindable");
    t.line("  ctrl-g (unreserved) -> accepted, so the two above are refused on their own merits");

    // ---- 1c. The class — every Ctrl-<letter>, every context ---------------
    t.section("1c. The class — no Ctrl-<letter> dispatches without catalog metadata");

    // The same property `tests/keymap_help_parity_test.rs` enforces over its
    // whole key universe, restated here over the axis this ticket widened.
    // A `Ctrl-<letter>` event can only be covered by a `CtrlChar` trigger:
    // `PrintableText` requires empty-or-Shift modifiers and every other trigger
    // matches a different `KeyCode`, so coverage is a single containment check.
    let mut dispatching = Vec::new();
    for &ctx in ALL_HELP_CONTEXTS {
        let context = context_for(ctx);
        let bindings = context_bindings(ctx);
        for ch in 'a'..='z' {
            let Some(action) = action_for_key(ctrl(ch), &context) else {
                continue;
            };
            let taught = bindings
                .iter()
                .any(|b| b.triggers.contains(&KeyTrigger::CtrlChar(ch)));
            assert!(
                taught,
                "{ctx:?}: Ctrl-{ch} dispatches {action:?} but no catalog binding teaches it \
                 — either it is a real binding missing its footer/help metadata, or the \
                 keymap arm it reached is modifier-blind"
            );
            dispatching.push(format!("  {ctx:?} Ctrl-{ch} -> {action:?}"));
        }
    }

    // The sweep must not be vacuous: the three Ctrl-<letter> chords the app
    // really binds have to still dispatch somewhere, or "no uncovered Ctrl key"
    // would be satisfied by a keymap that had lost every Ctrl binding it ever
    // had. (Ctrl-Space is a real binding too, but it is not a Ctrl-<letter>, so
    // it is outside the `'a'..='z'` sweep this control belongs to.)
    for (ctx, ch) in [
        (HelpContext::Home, 'c'),
        (HelpContext::Home, 't'),
        (HelpContext::LoggingList, 's'),
    ] {
        assert!(
            action_for_key(ctrl(ch), &context_for(ctx)).is_some(),
            "{ctx:?}: Ctrl-{ch} is a real binding and must still dispatch"
        );
    }
    t.line(&format!(
        "  {} Ctrl-<letter> dispatches survive the sweep, each with catalog metadata:",
        dispatching.len()
    ));
    for line in &dispatching {
        t.line(line);
    }
    t.line(
        "  Every other Ctrl-<letter>, in every context, is unbound. Run against the\n\
         earlier keymap, this same sweep trips on Home/Ctrl-L — the plain-`l`\n\
         Logging-workspace arm in `normal_key`, which sits ahead of every per-workspace\n\
         dispatch and so leaked in every normal-mode context.",
    );

    t.write("01-ctrl-key-parity.txt");
}

// ---------------------------------------------------------------------------
// 2. A Logging deep link cannot strand the viewer at `Loading`
// ---------------------------------------------------------------------------

fn config_row(name: &str, project: &str) -> GcloudConfiguration {
    GcloudConfiguration {
        name: GcloudConfigurationName::new(name),
        is_active: true,
        account: None,
        project: Some(project.to_string()),
        region: None,
        zone: None,
    }
}

/// Walks real navigation history — into Logging, away to Home, then `Back` —
/// so the restore fires through the same reducer path a deep link takes.
/// Nothing is hand-seeded.
fn navigate_back_into_logging(mut app: AppState) -> (AppState, Vec<Effect>) {
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Logging));
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Home));
    let effects = reduce(&mut app, &Action::Navigation(NavAction::Back));
    assert_eq!(app.root.workspace, Workspace::Logging);
    (app, effects)
}

/// Forecloses: the `NavLocation::Logging` restore arm marking the buffer
/// `Loading` and then calling a loader that dispatches nothing when no context
/// is resolved — a spinner for a fetch that never happened, which the tail poll
/// (skipped for the same reason) would never clear.
#[test]
fn a_logging_deep_link_never_spins_for_a_fetch_that_was_not_dispatched() {
    let mut t = Transcript::default();
    t.line(
        "Item 2 — restoring the Logging location through navigation history.\n\
         The status may read Loading only when a load was actually dispatched.",
    );

    // ---- No resolved context: nothing to load, so nothing to spin for -----
    t.section("2a. No resolved context — no effect, and the status settles to Initial");

    let (app, effects) = navigate_back_into_logging(AppState::new());
    assert!(
        app.root.current_context().is_none(),
        "this half of the evidence is about the no-context case"
    );
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::LoadRecentLogs { .. })),
        "with no context there is nothing to fetch: {effects:?}"
    );
    assert_eq!(
        app.root.logging.status,
        LoggingStatus::Initial,
        "the restore left the viewer spinning with nothing to clear it"
    );
    // Read through the projection too — the spinner is a rendered string, and
    // that is the form the operator actually sees.
    let projection = project_logging(&app.root);
    assert_eq!(
        projection.header.status, "Not loaded",
        "the header must say the buffer was never loaded, not that a load is in flight"
    );
    t.line(&format!(
        "  context=None  effects={:?}  status={:?}  header=\"{}\"",
        effects, app.root.logging.status, projection.header.status
    ));

    // ---- Context resolved: the load IS dispatched, so Loading is honest ---
    t.section("2b. Context resolved — the load is dispatched and Loading stands");

    // Without this half, "never strand at Loading" would be satisfied by a
    // viewer that never showed a spinner at all.
    let mut seeded = AppState::new();
    reduce(
        &mut seeded,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config_row("prod", "acme-prod")],
        }),
    );
    let (app, effects) = navigate_back_into_logging(seeded);
    assert!(app.root.current_context().is_some());
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadRecentLogs { .. })),
        "a restore with a context must ask for the logs: {effects:?}"
    );
    assert_eq!(
        app.root.logging.status,
        LoggingStatus::Loading,
        "a dispatched load must keep the spinner up until its result lands"
    );
    let projection = project_logging(&app.root);
    assert_eq!(projection.header.status, "Loading...");
    t.line(&format!(
        "  context=Some  LoadRecentLogs dispatched  status={:?}  header=\"{}\"",
        app.root.logging.status, projection.header.status
    ));

    t.write("02-logging-deep-link.txt");
}

// ---------------------------------------------------------------------------
// 3. The search filter compiles once per apply, and the label agrees with it
// ---------------------------------------------------------------------------

fn log_entry(message: &str) -> LogEntry {
    LogEntry {
        insert_id: message.to_string(),
        timestamp: "2026-07-27T10:00:00Z".to_string(),
        severity: LogSeverity::Error,
        resource_type: "gce_instance".to_string(),
        resource_labels: Default::default(),
        log_name: "projects/acme-prod/logs/stdout".to_string(),
        message: message.to_string(),
        payload: LogPayload::Empty,
        labels: Default::default(),
    }
}

/// The corpus the applied filters are asked about. Chosen so the patterns
/// below each keep a different subset: no two applies can look alike.
const CORPUS: [&str; 4] = [
    "[ERROR] nginx: upstream returned 502",
    "[INFO] nginx: reloaded configuration",
    "[ERROR] payments: database pool exhausted",
    "[DEBUG] cache warm-up finished",
];

/// True when `message`'s corpus entry passes `filter` the way
/// `LoggingState::visible_entries` applies it — over the message, the severity
/// label and the resource type, not the message alone.
///
/// Deriving the expectation from the same three haystacks matters: every corpus
/// entry currently shares `Error` and `gce_instance`, so a message-only
/// expectation happens to agree today and would start contradicting a correct
/// implementation the moment the corpus stopped being uniform.
fn corpus_entry_matches(filter: &TextFilter, message: &str) -> bool {
    let entry = log_entry(message);
    filter.matches_any([
        entry.message.as_str(),
        entry.severity.label(),
        entry.resource_type.as_str(),
    ])
}

/// A Logging root with the corpus loaded and `pattern` applied through the same
/// draft-then-apply path the `/` key drives.
fn logging_root_with(pattern: &str) -> RootState {
    let mut root = RootState {
        workspace: Workspace::Logging,
        ..Default::default()
    };
    root.logging
        .apply_recent(CORPUS.iter().map(|m| log_entry(m)).collect(), 0);
    root.logging.filter_draft = pattern.to_string();
    root.logging.apply_filter();
    root
}

/// Forecloses: the header's mode label — now derived from the pure-syntax view
/// with no regex compile — describing a different pattern than the rows were
/// matched against; and the cached compiled filter outliving the source it was
/// built from once `applied_filter` stopped being re-parsed on every call.
#[test]
fn the_search_label_and_the_rows_always_describe_the_same_pattern() {
    let mut t = Transcript::default();
    t.line(
        "Item 3 — the log search compiles once per apply. The header's mode label\n\
         is derived without compiling at all, so what is pinned here is that the\n\
         no-compile label and the compiled filter still agree, and that the cached\n\
         compile cannot outlive its source.",
    );

    // ---- 3a. The label agrees with the compiled parse ---------------------
    t.section("3a. The uncompiled label agrees with the compiled filter, across the grammar");

    // One input per branch of the grammar, each in both polarities, plus the
    // three spellings of match-all (which name no mode: they narrow nothing,
    // so a mode label would advertise a filter the rows do not reflect).
    //
    // The `! /…/` spellings are the ones with teeth. The grammar re-trims after
    // stripping `!`, so `! /err/` is an inverted *regex*; a label derived by
    // walking the string a second time — the shape this fix removed and the
    // shape a future "simplification" would put back — reads it as an inverted
    // substring search for a space followed by `/err/`, and the header then
    // names a mode the rows were never matched against.
    let patterns = [
        "nginx",
        "!nginx",
        "! nginx",
        "~ngx",
        "!~ngx",
        "! ~ngx",
        "/ngi?nx/",
        "!/ngi?nx/",
        "! /ngi?nx/",
        "[ERROR]",
        "",
        "  ",
        "//",
        "~",
        "!",
        "!//",
        "!~",
        "! //",
    ];
    for pattern in patterns {
        let root = logging_root_with(pattern);
        let header = project_logging(&root).header;
        let syntax = TextFilterSyntax::of(pattern);
        let compiled = TextFilter::parse(pattern)
            .unwrap_or_else(|error| panic!("{pattern:?} must compile here: {error}"));

        // Match-all is derived twice, independently: the syntax view computes
        // `!inverted && needle.is_empty()`, the compiled filter asks its
        // `Predicate`. They can disagree, so this is worth asserting.
        //
        // Kind and inversion are *not* asserted against each other here, and
        // deliberately so: `TextFilter::parse` is literally `TextFilterSyntax::of`
        // plus a matcher build — it copies `syntax.kind` into the filter and
        // hands `syntax.inverted` to the predicate — so comparing the two
        // getters compares a value to itself and could not fail under any
        // mutation of this ticket's fix. That the two views share one decision
        // is the grammar reconciliation's property, guarded there. What this loop pins is below:
        // the projection's no-compile label, which is a genuinely separate
        // derivation, against the filter the rows were matched with.
        assert_eq!(
            syntax.is_match_all(),
            compiled.is_match_all(),
            "{pattern:?}: the label and the compiled filter must agree on whether the \
             pattern narrows anything at all"
        );

        // The load-bearing assertion: the label the header actually shows.
        let expected = if pattern.trim().is_empty() || compiled.is_match_all() {
            None
        } else {
            Some(compiled.describe())
        };
        assert_eq!(
            header.search_mode, expected,
            "{pattern:?}: the header names a mode the compiled filter does not"
        );
        assert_eq!(
            header.search_error, None,
            "{pattern:?} compiles, so the header must show no error"
        );
        t.line(&format!(
            "  {pattern:<10} | kind={:<9?} inverted={:<5} match_all={:<5} | header={:?}",
            syntax.kind(),
            syntax.is_inverted(),
            syntax.is_match_all(),
            header.search_mode
        ));
    }

    // The block above would be vacuous if every pattern produced the same
    // answer, so pin that the corpus of labels is actually varied.
    let labels: Vec<Option<String>> = patterns
        .iter()
        .map(|p| project_logging(&logging_root_with(p)).header.search_mode)
        .collect();
    for expected in [
        "substring",
        "not substring",
        "fuzzy",
        "not fuzzy",
        "regex",
        "not regex",
    ] {
        assert!(
            labels.contains(&Some(expected.to_string())),
            "the pattern corpus must exercise the {expected:?} label"
        );
    }
    assert!(
        labels.contains(&None),
        "the pattern corpus must exercise the no-label case"
    );

    // ---- 3b. The one legitimate divergence, pinned ------------------------
    t.section("3b. `/[a/` — syntactically a regex, and still no mode label");

    // Syntax alone cannot know a regex fails to compile, which is the single
    // place the two views are allowed to differ. The header resolves it by
    // showing the error and NO mode: it never claims a mode the rows do not
    // reflect.
    let root = logging_root_with("/[a/");
    let header = project_logging(&root).header;
    assert_eq!(
        TextFilterSyntax::of("/[a/").kind(),
        TextFilterKind::Regex,
        "`/[a/` is syntactically a regex — that is exactly why the label path needs a \
         second signal to suppress it"
    );
    assert!(
        TextFilter::parse("/[a/").is_err(),
        "`/[a/` must not compile, or this case proves nothing"
    );
    assert_eq!(
        header.search_mode, None,
        "an unparseable pattern has no mode to report"
    );
    let error = header
        .search_error
        .clone()
        .expect("the header must show the compile error instead");
    assert!(
        error.contains("unclosed character class"),
        "the error must name the cause: {error}"
    );
    assert!(
        root.logging.visible_entries().is_empty(),
        "an unparseable pattern shows no rows, never silently everything"
    );
    t.line(&format!(
        "  /[a/  syntax kind=Regex | header mode={:?} error={error:?} | rows=0",
        header.search_mode
    ));

    // ---- 3c. The cached compile cannot outlive its source -----------------
    t.section("3c. Applying a new pattern changes which rows come back");

    // `applied_filter` is now a private newtype holding source + compiled
    // form. If an apply ever updated one without the other, the rows would
    // keep answering to the previous pattern while the header showed the new
    // one. Three applies in a row on one state, each with a different answer.
    let mut root = RootState {
        workspace: Workspace::Logging,
        ..Default::default()
    };
    root.logging
        .apply_recent(CORPUS.iter().map(|m| log_entry(m)).collect(), 0);

    let mut seen: Vec<(String, Vec<String>)> = Vec::new();
    for pattern in ["", "nginx", "[ERROR]", "!nginx", "~dbpool"] {
        root.logging.filter_draft = pattern.to_string();
        root.logging.apply_filter();
        assert_eq!(
            root.logging.applied_filter(),
            pattern,
            "the accessor must report the source that was just applied"
        );
        let rows: Vec<String> = root
            .logging
            .visible_entries()
            .into_iter()
            .map(|entry| entry.message.clone())
            .collect();
        // Every row that comes back must satisfy the pattern that is applied
        // *now* — a stale compiled filter fails here even though the source
        // accessor above would still look correct.
        let filter = TextFilter::parse(pattern).expect("corpus patterns compile");
        for row in &rows {
            assert!(
                corpus_entry_matches(&filter, row),
                "{pattern:?}: row {row:?} came back but does not match the applied pattern"
            );
        }
        let expected: Vec<&&str> = CORPUS
            .iter()
            .filter(|message| corpus_entry_matches(&filter, message))
            .collect();
        assert_eq!(
            rows.len(),
            expected.len(),
            "{pattern:?}: expected {expected:?}, got {rows:?}"
        );
        seen.push((pattern.to_string(), rows));
    }

    // The applies must actually disagree with each other, or "the rows follow
    // the pattern" would hold for a filter that ignored the pattern entirely.
    assert_eq!(
        seen[0].1.len(),
        CORPUS.len(),
        "the empty pattern hides nothing"
    );
    assert_eq!(seen[1].1.len(), 2, "`nginx` keeps the two nginx lines");
    assert_eq!(
        seen[2].1.len(),
        2,
        "`[ERROR]` keeps the two bracketed errors"
    );
    assert_eq!(seen[3].1.len(), 2, "`!nginx` keeps the two non-nginx lines");
    assert_eq!(seen[4].1.len(), 1, "`~dbpool` finds the database pool line");
    assert_ne!(
        seen[1].1, seen[2].1,
        "two different patterns must keep two different row sets, or the sequence above \
         could not detect a filter left behind by the previous apply"
    );
    for (pattern, rows) in &seen {
        t.line(&format!(
            "  applied {pattern:<10} -> {} rows {rows:?}",
            rows.len()
        ));
    }

    t.write("03-search-filter.txt");
}
