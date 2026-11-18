//! Evidence generator — the seven keymap defects the
//! modifier-complete parity sweep found, and the one modifier policy it
//! deliberately did not touch.
//!
//! Before this ticket, `tests/keymap_help_parity_test.rs` probed three
//! hand-picked Ctrl chords. A hand-picked probe list can only find the leak
//! somebody already suspected, so it found none of what is below. Widening the
//! sweep to the full cross-product — every probe key crossed with `CONTROL`,
//! `ALT`, `CONTROL|ALT` and `SUPER`, each also crossed with `SHIFT` — turned up
//! seven defects, in two distinct classes:
//!
//! **Class one: chord leaks (six).** An arm guarded with
//! `key.modifiers.contains(CONTROL)` rather than an exact match admits
//! `Ctrl-Alt-<key>` and `Ctrl-Shift-<key>` as well, and an arm with no modifier
//! guard at all admits everything. So `Ctrl-Alt-C` force-quit the app,
//! `Ctrl-Alt-T` opened Tunnels, `Ctrl-Alt-S` saved the log buffer,
//! `Ctrl-Alt-Left` walked navigation history, in the resource list every
//! chorded `Left`/`Right` moved the column cursor, and — the last one found, in
//! `override_trigger` — every `ctrl-<x>` hotkey a *user* configured answered to
//! `Ctrl-Alt-<x>` and `Ctrl-Shift-<x>` too. None of these chords appears in the
//! footer or the help overlay: the operator had no way to learn they existed,
//! and no way to understand what had happened when one fired by accident.
//! `Ctrl-Alt-C` is the sharp one — an unlabelled chord that quits.
//!
//! **Class two: a half-dead arm (one).** `Char(' ')` in `resource_content_key`
//! guarded with `key.modifiers.is_empty()` where every sibling arm uses
//! `is_plain_text_key`. `is_plain_text_key` also admits `SHIFT`, which terminals
//! report alongside a character under the enhanced keyboard protocols, so
//! `Shift-Space` refused to mark a row while the footer went on advertising
//! "Space". This is the opposite failure from class one: not a binding the
//! catalog never claimed, but a binding the catalog claimed and the keymap only
//! half-honoured. A catalog entry cannot express "honoured bare, refused with
//! Shift", so no amount of catalog metadata could have warned about it.
//!
//! Every leak below is recorded twice: the chord that must no longer dispatch,
//! and the intended binding that must still fire. A fix that took the real key
//! with it would satisfy the first half and fail the second, and an evidence
//! file that only recorded removals could not tell the two apart.
//!
//! ## What was deliberately NOT changed, and why there is no follow-up ticket
//!
//! All 93 non-`Char` arms in `src/input/keymap.rs` — `Enter`, `Esc`, `Tab`,
//! `BackTab`, `Up`, `Down`, `PageUp`, `PageDown`, `Backspace` — match on
//! `key.code` alone, so they answer to any modifier. That is a policy, not an
//! oversight, and guarding them was considered and rejected: terminals speaking
//! the kitty keyboard protocol report stray modifiers on `Esc` and `Enter`, so a
//! strict guard would leave the escape hatch dead in those terminals. The help
//! overlay, the confirmations and the previews would stop closing for real
//! users, and the only symptom would be a key that does nothing.
//!
//! This is a **closed decision with no follow-up ticket**, deliberately not
//! filed. What protects it is not an allowlist — an allowlist would excuse the
//! behaviour and guarantee the suite could never fail on it — but the positive
//! invariant `control_keys_ignore_modifiers` in
//! `tests/keymap_help_parity_test.rs`, which asserts that each of those nine
//! codes dispatches the *same* action under every chord, in every `HelpContext`.
//! The day somebody adds a `Ctrl-Enter` binding without catalog metadata, that
//! invariance breaks and the test fails naming the context and the key.
//!
//! Every assertion here runs unconditionally. When `NUVEM_EVIDENCE_DIR` is set,
//! each section also writes a transcript of what it observed; with the variable
//! unset the test writes nothing at all and never writes outside that directory.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

use nuvem::app::projection::binding::HelpContext;
use nuvem::app::{Action, LoggingAction, NavAction, ResourcesAction, WorkspaceAction};
use nuvem::config::hotkeys::KeyOverrides;
use nuvem::input::Context;
use nuvem::input::keymap::action_for_key;

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
/// tests can run concurrently without sharing state.
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

fn chord(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent {
        code,
        modifiers,
        kind: KeyEventKind::Press,
        state: KeyEventState::empty(),
    }
}

/// A readable spelling of a chord, for the transcript and the failure messages:
/// `Ctrl-Alt-C`, `Shift-Left`, `Space`.
fn describe(ev: &KeyEvent) -> String {
    let mut parts = Vec::new();
    if ev.modifiers.contains(KeyModifiers::CONTROL) {
        parts.push("Ctrl".to_string());
    }
    if ev.modifiers.contains(KeyModifiers::ALT) {
        parts.push("Alt".to_string());
    }
    if ev.modifiers.contains(KeyModifiers::SHIFT) {
        parts.push("Shift".to_string());
    }
    if ev.modifiers.contains(KeyModifiers::SUPER) {
        parts.push("Super".to_string());
    }
    parts.push(match ev.code {
        KeyCode::Char(' ') => "Space".to_string(),
        KeyCode::Char(c) => c.to_uppercase().to_string(),
        other => format!("{other:?}"),
    });
    parts.join("-")
}

/// Asserts a chord dispatches nothing in `context`, and records it. `note` says
/// what the chord meant before the fix — or, for a chord that was never bound,
/// why it is asserted here anyway.
fn assert_unbound(
    t: &mut Transcript,
    ctx: HelpContext,
    context: &Context,
    ev: KeyEvent,
    note: &str,
) {
    let action = action_for_key(ev, context);
    assert_eq!(
        action,
        None,
        "{ctx:?}: {} must be unbound — {note}. No footer or help entry teaches this chord",
        describe(&ev)
    );
    t.line(&format!(
        "  {ctx:?}  {:<16} -> (unbound)   [{note}]",
        describe(&ev)
    ));
}

/// Asserts a key still dispatches the action it is meant to, and records it.
fn assert_dispatches(
    t: &mut Transcript,
    ctx: HelpContext,
    context: &Context,
    ev: KeyEvent,
    expected: Action,
) {
    assert_eq!(
        action_for_key(ev, context),
        Some(expected.clone()),
        "{ctx:?}: {} is a real binding and must still dispatch {expected:?} — the guard was \
         added to reject the chord, not to delete the key",
        describe(&ev)
    );
    t.line(&format!("  {ctx:?}  {:<16} -> {expected:?}", describe(&ev)));
}

// ---------------------------------------------------------------------------
// 1. Leak: Ctrl-C's force-quit answered to Ctrl-Alt-C and Ctrl-Shift-C
// ---------------------------------------------------------------------------

/// Forecloses: `action_for_key`'s force-quit arm guarding with
/// `modifiers.contains(CONTROL)`, so two unlabelled chords quit the app.
#[test]
fn only_ctrl_c_force_quits() {
    let mut t = Transcript::default();
    t.line(
        "Leak 1 — the force-quit arm sits ahead of every mode dispatch, so its\n\
         modifier guard is the only thing between an accidental chord and losing the\n\
         session. It answered to Ctrl-Alt-C and Ctrl-Shift-C as well as Ctrl-C.",
    );

    // Quit is dispatched before the mode match, so it reaches identically from
    // every context. Home and Help are probed as the two extremes: the default
    // view and a modal overlay with its own key table.
    for ctx in [HelpContext::Home, HelpContext::Help] {
        let context = context_for(ctx);
        t.section(&format!("{ctx:?}"));
        assert_unbound(
            &mut t,
            ctx,
            &context,
            chord(
                KeyCode::Char('c'),
                KeyModifiers::CONTROL | KeyModifiers::ALT,
            ),
            "it used to force-quit the app",
        );
        assert_unbound(
            &mut t,
            ctx,
            &context,
            chord(
                KeyCode::Char('c'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT,
            ),
            "it used to force-quit the app",
        );
        // The paired positive: the guard rejects the chords, it does not delete
        // the escape hatch. Without this line the two above would be satisfied
        // by a keymap that had lost force-quit entirely.
        assert_dispatches(
            &mut t,
            ctx,
            &context,
            chord(KeyCode::Char('c'), KeyModifiers::CONTROL),
            Action::QuitRequested,
        );
    }

    t.write("01-ctrl-c-force-quit.txt");
}

// ---------------------------------------------------------------------------
// 2 & 3. Leaks: Ctrl-T (Tunnels) and Ctrl-S (Logging save)
// ---------------------------------------------------------------------------

/// Forecloses: the `Ctrl-T` Tunnels arm and the `Ctrl-S` log-save arm accepting
/// `Ctrl-Alt-` and `Ctrl-Shift-` variants the catalog never claimed.
#[test]
fn ctrl_t_and_ctrl_s_are_exact_chords() {
    let mut t = Transcript::default();
    t.line(
        "Leaks 2 and 3 — the two named Ctrl-<letter> bindings. Each is resolved\n\
         ahead of the plain-letter arm sharing its character (bare `t` is the Logging\n\
         tail toggle, bare `s` the severity floor), so each carries a modifier guard;\n\
         both guards were inexact and admitted Ctrl-Alt- and Ctrl-Shift- variants.",
    );

    // ---- Ctrl-T opens the Tunnels workspace -------------------------------
    t.section("2. Ctrl-T — Tunnels workspace (reachable from every normal-mode context)");

    let home = context_for(HelpContext::Home);
    assert_unbound(
        &mut t,
        HelpContext::Home,
        &home,
        chord(
            KeyCode::Char('t'),
            KeyModifiers::CONTROL | KeyModifiers::ALT,
        ),
        "it used to open the Tunnels workspace",
    );
    assert_unbound(
        &mut t,
        HelpContext::Home,
        &home,
        chord(
            KeyCode::Char('t'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ),
        "it used to open the Tunnels workspace",
    );
    assert_dispatches(
        &mut t,
        HelpContext::Home,
        &home,
        chord(KeyCode::Char('t'), KeyModifiers::CONTROL),
        Action::Workspace(WorkspaceAction::Tunnels),
    );
    // The key the Ctrl-T guard exists to protect: bare `t` in the Logging list
    // must still toggle the tail, or the guard would have been unnecessary.
    let logging = context_for(HelpContext::LoggingList);
    assert_dispatches(
        &mut t,
        HelpContext::LoggingList,
        &logging,
        press(KeyCode::Char('t')),
        Action::Logging(LoggingAction::ToggleTail),
    );

    // ---- Ctrl-S saves the visible log buffer ------------------------------
    t.section("3. Ctrl-S — save the visible log buffer (Logging list only)");

    assert_unbound(
        &mut t,
        HelpContext::LoggingList,
        &logging,
        chord(
            KeyCode::Char('s'),
            KeyModifiers::CONTROL | KeyModifiers::ALT,
        ),
        "it used to save the visible log buffer",
    );
    assert_unbound(
        &mut t,
        HelpContext::LoggingList,
        &logging,
        chord(
            KeyCode::Char('s'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ),
        "it used to save the visible log buffer",
    );
    assert_dispatches(
        &mut t,
        HelpContext::LoggingList,
        &logging,
        chord(KeyCode::Char('s'), KeyModifiers::CONTROL),
        Action::Logging(LoggingAction::SaveToFile),
    );
    assert_dispatches(
        &mut t,
        HelpContext::LoggingList,
        &logging,
        press(KeyCode::Char('s')),
        Action::Logging(LoggingAction::RaiseSeverityFloor),
    );

    t.write("02-3-ctrl-t-and-ctrl-s.txt");
}

// ---------------------------------------------------------------------------
// 4 & 5. Leaks: the history keys and the ResourceList column cursor
// ---------------------------------------------------------------------------

/// Forecloses: `Alt-Left`/`Alt-Right` history navigation answering to
/// `Ctrl-Alt-Left` and `Alt-Shift-Left`, and the resource list's column cursor
/// answering to every chorded `Left`/`Right`.
///
/// These two live in one test because they are the same two key codes seen from
/// two arms: `normal_key`'s history branch runs first and `resource_content_key`
/// picks up what falls through, so a guard added to either changes what the
/// other sees. `Left` and `Right` are for that reason the two codes deliberately
/// excluded from `control_keys_ignore_modifiers` — on them the modifier is
/// load-bearing, and this is where that is pinned instead.
#[test]
fn left_and_right_mean_exactly_two_things() {
    let mut t = Transcript::default();
    t.line(
        "Leaks 4 and 5 — Left/Right carry two meanings: with Alt they walk\n\
         navigation history, bare they move the resource list's column cursor. Every\n\
         other chord on those codes must mean nothing at all, or the operator gets one\n\
         of the two meanings at random.",
    );

    // ---- 4. History: exactly ALT ------------------------------------------
    t.section("4. Alt-Left / Alt-Right — navigation history");

    let home = context_for(HelpContext::Home);
    for (code, was) in [
        (KeyCode::Left, "it used to navigate back through history"),
        (
            KeyCode::Right,
            "it used to navigate forward through history",
        ),
    ] {
        assert_unbound(
            &mut t,
            HelpContext::Home,
            &home,
            chord(code, KeyModifiers::CONTROL | KeyModifiers::ALT),
            was,
        );
        assert_unbound(
            &mut t,
            HelpContext::Home,
            &home,
            chord(code, KeyModifiers::ALT | KeyModifiers::SHIFT),
            was,
        );
    }
    assert_dispatches(
        &mut t,
        HelpContext::Home,
        &home,
        chord(KeyCode::Left, KeyModifiers::ALT),
        Action::Navigation(NavAction::Back),
    );
    assert_dispatches(
        &mut t,
        HelpContext::Home,
        &home,
        chord(KeyCode::Right, KeyModifiers::ALT),
        Action::Navigation(NavAction::Forward),
    );

    // ---- 5. The column cursor: exactly no modifiers ------------------------
    t.section("5. ResourceList Left / Right — the column cursor");

    // The resource list is where the two meanings actually collide: history is
    // resolved in `normal_key` before the list ever sees the key. So both
    // halves are asserted here, in the one context that has both.
    let list = context_for(HelpContext::ResourceList);
    for (code, was) in [
        (KeyCode::Left, "it used to move the column cursor left"),
        (KeyCode::Right, "it used to move the column cursor right"),
    ] {
        for modifiers in [
            KeyModifiers::CONTROL,
            KeyModifiers::SHIFT,
            KeyModifiers::SUPER,
        ] {
            assert_unbound(
                &mut t,
                HelpContext::ResourceList,
                &list,
                chord(code, modifiers),
                was,
            );
        }
    }
    assert_dispatches(
        &mut t,
        HelpContext::ResourceList,
        &list,
        press(KeyCode::Left),
        Action::Resources(ResourcesAction::SelectPreviousColumn),
    );
    assert_dispatches(
        &mut t,
        HelpContext::ResourceList,
        &list,
        press(KeyCode::Right),
        Action::Resources(ResourcesAction::SelectNextColumn),
    );
    // Alt-Left/Alt-Right must still reach history *from the resource list*, the
    // context where the column cursor would otherwise swallow them.
    assert_dispatches(
        &mut t,
        HelpContext::ResourceList,
        &list,
        chord(KeyCode::Left, KeyModifiers::ALT),
        Action::Navigation(NavAction::Back),
    );
    assert_dispatches(
        &mut t,
        HelpContext::ResourceList,
        &list,
        chord(KeyCode::Right, KeyModifiers::ALT),
        Action::Navigation(NavAction::Forward),
    );

    t.write("04-5-history-and-column-cursor.txt");
}

// ---------------------------------------------------------------------------
// 6. The half-dead arm: Shift-Space refused to mark
// ---------------------------------------------------------------------------

/// Forecloses: `resource_content_key`'s `Char(' ')` arm guarding with
/// `key.modifiers.is_empty()` instead of `is_plain_text_key`.
///
/// This is not a chord leak and is not recorded as one. The other five were
/// bindings the catalog never claimed; this was a binding the catalog *did*
/// claim — the footer advertises "Space" — and the keymap only half-honoured,
/// dispatching the bare event and refusing the SHIFT-carrying one that terminals
/// send under the enhanced keyboard protocols. A catalog entry has no way to say
/// "honoured bare, refused with Shift", so this class is invisible to the
/// catalog-parity check in both directions; it is caught instead by
/// `half_dead_char_trigger` in `tests/keymap_help_parity_test.rs`.
#[test]
fn space_marks_a_row_whether_or_not_shift_rides_along() {
    let mut t = Transcript::default();
    t.line(
        "Defect 6 — a different class from the five chord leaks. The footer\n\
         advertises Space to mark a row; the arm behind it accepted only the bare\n\
         event, so Shift-Space did nothing in terminals that report SHIFT alongside a\n\
         character. Nothing in the catalog could have expressed the difference.",
    );

    let list = context_for(HelpContext::ResourceList);
    t.section("Space marks, with or without Shift");

    assert_dispatches(
        &mut t,
        HelpContext::ResourceList,
        &list,
        press(KeyCode::Char(' ')),
        Action::Resources(ResourcesAction::ToggleMark),
    );
    assert_dispatches(
        &mut t,
        HelpContext::ResourceList,
        &list,
        chord(KeyCode::Char(' '), KeyModifiers::SHIFT),
        Action::Resources(ResourcesAction::ToggleMark),
    );

    // The control: `is_plain_text_key` admits SHIFT and nothing else, so the fix
    // widened the arm by exactly one modifier. Ctrl-Space is still unbound — had
    // the arm simply dropped its guard, the line above would pass and this one
    // would not.
    t.section("...and only Shift: the arm was widened by one modifier, not unguarded");
    for modifiers in [
        KeyModifiers::CONTROL,
        KeyModifiers::ALT,
        KeyModifiers::SUPER,
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    ] {
        assert_unbound(
            &mut t,
            HelpContext::ResourceList,
            &list,
            chord(KeyCode::Char(' '), modifiers),
            "is_plain_text_key admits SHIFT and nothing else, so marking stays chord-free",
        );
    }

    t.write("06-shift-space-marks.txt");
}

// ---------------------------------------------------------------------------
// 7. Leak: the user-configurable Ctrl-<char> hotkey path
// ---------------------------------------------------------------------------

/// Forecloses: `override_trigger`'s Ctrl branch guarding with
/// `key.modifiers.contains(CONTROL)` instead of an exact match.
///
/// This is the same class as leaks 1-3 but in the one path the *operator*
/// controls. `override_trigger` is where a `KeyEvent` becomes the `KeyTrigger`
/// looked up in the effective hotkey table, so a `.contains` guard there widens
/// *every* user-configured `ctrl-<x>` binding into `Ctrl-Alt-<x>` and
/// `Ctrl-Shift-<x>` at once — a leak that grows with the user's config rather
/// than with the app's own key table. The catalog side was already exact
/// (`trigger_matches_event(CtrlChar)` requires `modifiers == CONTROL`), so
/// keymap and catalog disagreed: the chords fired and nothing advertised them.
#[test]
fn ctrl_char_overrides_are_exact_chords() {
    let mut t = Transcript::default();
    t.line(
        "Leak 7 — the user-configurable hotkey path. `override_trigger` maps a\n\
         KeyEvent to the KeyTrigger looked up in the effective hotkey table; its Ctrl\n\
         branch matched with `.contains(CONTROL)`, so every `ctrl-<x>` a user binds also\n\
         answered to Ctrl-Alt-<x> and Ctrl-Shift-<x>, chords no footer or help ever\n\
         claims and no config ever asked for.",
    );

    // `ctrl-r` for refresh: `ctrl-c`, `ctrl-s` and `ctrl-t` are reserved and
    // rejected by validation, so a *valid* config is what has to be probed here.
    let overrides = KeyOverrides::from_file(&BTreeMap::from([(
        "refresh".to_string(),
        "ctrl-r".to_string(),
    )]))
    .expect("ctrl-r is a valid, non-reserved override target");

    let ctx = HelpContext::Home;
    let mut context = context_for(ctx);
    context.hotkeys = overrides;

    t.section("7. A user-configured Ctrl-R refresh answers to Ctrl-R and to nothing else");

    for modifiers in [
        KeyModifiers::CONTROL | KeyModifiers::ALT,
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    ] {
        assert_unbound(
            &mut t,
            ctx,
            &context,
            chord(KeyCode::Char('r'), modifiers),
            "it used to fire the user's ctrl-r refresh binding",
        );
    }

    // The paired positive: the guard rejects the chords, it does not break the
    // user's binding. Without this line the two above would be satisfied by an
    // `override_trigger` that had stopped yielding `CtrlChar` at all.
    assert_dispatches(
        &mut t,
        ctx,
        &context,
        chord(KeyCode::Char('r'), KeyModifiers::CONTROL),
        Action::Workspace(WorkspaceAction::Home),
    );

    t.write("07-ctrl-char-overrides.txt");
}
