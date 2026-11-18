//! Airtight parity tests proving that `binding::context_bindings` (the
//! footer/help catalog) is a faithful single source of truth for what
//! `keymap::action_for_key` actually dispatches, across every
//! `HelpContext`.
//!
//! Four properties are enforced, all across *every* `HelpContext` (not just
//! the default view):
//!
//! 1. `every_dispatched_key_has_catalog_metadata` — every key that the
//!    keymap dispatches (`action_for_key(..).is_some()`) in a context has a
//!    matching catalog `Binding` for that context. A future keymap key added
//!    without catalog metadata makes this test fail.
//! 2. `no_orphan_catalog_bindings` — every catalog binding for a context has
//!    at least one trigger that actually dispatches in that context, so the
//!    catalog never claims a key that keymap doesn't honor; and no `Char`
//!    trigger is *half-dead*, i.e. honored bare but refused with `SHIFT`, which
//!    is the failure mode a catalog entry cannot express and so cannot warn
//!    about.
//! 3. `control_keys_ignore_modifiers` — the nine control keys dispatch the same
//!    action under every chord, which is a deliberate policy (see the test) and
//!    is asserted rather than excused, so a chord binding added to one of them
//!    without catalog metadata breaks the invariance and fails here.
//! 4. `plain_text_arms_reject_chord_modifiers` — `Alt-<letter>` and
//!    `Ctrl-<digit>`, the two named blind spots, dispatch nothing
//!    anywhere; the property rests on a per-arm `is_plain_text_key` guard that
//!    is one edit away from being dropped.

use std::collections::BTreeMap;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use nuvem::app::projection::binding::{
    ALL_HELP_CONTEXTS, Binding, DisplayBinding, HelpContext, KeyTrigger, context_bindings,
    context_bindings_with, help_model_with,
};
use nuvem::app::state::root::RootState;
use nuvem::app::{Action, ResourcesAction, SystemAction};
use nuvem::config::hotkeys::KeyOverrides;
use nuvem::input::keymap::action_for_key;

use crate::support::keymap_context;

use keymap_context::context_for;

fn press(code: KeyCode) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: KeyModifiers::empty(),
        kind: KeyEventKind::Press,
        state: KeyEventState::empty(),
    }
}

fn modified_press(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent {
        code,
        modifiers,
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

/// The number of printable ASCII characters, `0x20` (space) through `0x7E`
/// (tilde) inclusive.
const PRINTABLE_ASCII_LEN: usize = 0x7E - 0x20 + 1;

const fn printable_ascii() -> [char; PRINTABLE_ASCII_LEN] {
    let mut chars = [' '; PRINTABLE_ASCII_LEN];
    let mut i = 0;
    while i < PRINTABLE_ASCII_LEN {
        chars[i] = (0x20 + i as u8) as char;
        i += 1;
    }
    chars
}

/// Every probe character the sweep uses, plain and under every chord.
///
/// This is **the complete printable ASCII range, `0x20..=0x7E`** — computed,
/// not enumerated — and that is the entire point of it. It is deliberately
/// *not* a list of the characters the keymap happens to bind today, nor a list
/// anybody chose: a curated alphabet only probes the characters somebody
/// already thought of, so the next keymap arm added on `+`, `=`, `*`, `<`, `@`
/// or `_` would dispatch through the whole chord space with zero sweep
/// coverage and no test failure. Do not "tidy" this back into a literal list,
/// and do not derive it by parsing `src/input/keymap.rs` — a harness that reads
/// the source it checks passes because it agrees with the code rather than
/// because the code is correct.
const PROBE_CHARS: &[char] = &printable_ascii();

/// The probe characters a terminal reports *with* the `SHIFT` modifier set.
///
/// The rule, written down so it can be argued with: **a character is listed
/// here iff producing it on a US keyboard layout requires holding Shift** —
/// the uppercase letters, and the shifted face of the digit and punctuation
/// keys (`!` for Shift+1, `:` for Shift+;, `?` for Shift+/, and so on).
/// Pressing Shift+j yields `Char('J')` carrying `SHIFT`; pressing Shift+;
/// yields `Char(':')` carrying `SHIFT`. These are swept as plain keys with
/// modifiers exactly `SHIFT`, because that is a real event, unlike
/// lowercase-plus-SHIFT, which no terminal produces and which is therefore not
/// modelled here.
///
/// The rule is **layout-specific**: on a layout where a different set of
/// characters sits behind Shift (`@` and `"` swap on a UK layout, AltGr
/// produces punctuation on many European ones), a terminal reports `SHIFT` on
/// characters this list does not name. That is a known limit of this axis, not
/// an oversight — recorded so that a future non-US bug report about a
/// SHIFT-carrying punctuation key is legible rather than mysterious.
const SHIFT_PROBE_CHARS: &[char] = &[
    'A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'I', 'J', 'K', 'L', 'M', 'N', 'O', 'P', 'Q', 'R', 'S',
    'T', 'U', 'V', 'W', 'X', 'Y', 'Z', '!', '"', '#', '$', '%', '&', '(', ')', '*', '+', ':', '<',
    '>', '?', '@', '^', '_', '{', '|', '}', '~',
];

/// Every non-`Char` key code the sweep uses, plain and under every chord.
const PROBE_CODES: &[KeyCode] = &[
    KeyCode::Enter,
    KeyCode::Esc,
    KeyCode::Tab,
    KeyCode::BackTab,
    KeyCode::Up,
    KeyCode::Down,
    KeyCode::Left,
    KeyCode::Right,
    KeyCode::PageUp,
    KeyCode::PageDown,
    KeyCode::Backspace,
];

/// The fixed probe set: covers every dispatch path reachable via
/// `key_code`/modifier combinations used anywhere in `keymap.rs`.
///
/// The `Ctrl-<letter>` sweep is deliberately **exhaustive** (`ctrl-a` ..
/// `ctrl-z`) rather than a hand-picked list of the Ctrl keys the app means to
/// bind. Most keymap arms match on `key.code` alone, so a `Ctrl-<letter>` leaks
/// into whatever plain-letter arm shares its character unless that arm guards
/// with `is_plain_text_key`. A curated probe list can only find the leaks
/// somebody already thought of; the full sweep is what catches the *next* Ctrl
/// binding added without catalog metadata, and the next plain-letter arm that
/// forgets its guard.
///
/// The same argument applies to the rest of the chord space, so the sweep is
/// exhaustive there too: every probe character *and* every non-`Char` code is
/// crossed with `CONTROL`, `ALT`, `CONTROL|ALT` and `SUPER`, each of those also
/// crossed with `SHIFT`. A keymap arm that guards with `.contains(CONTROL)`
/// rather than an exact match admits `Ctrl-Alt-<key>` and `Ctrl-Shift-<key>`
/// too; a curated chord list only finds the chord somebody already suspected,
/// while the full cross-product finds the next one.
///
/// `SHIFT` alone is handled differently on the two halves of the probe set,
/// because a terminal reports it differently:
///
/// * On **characters** it is character-forming, not a chord: pressing Shift+j
///   arrives as `Char('J')` carrying `SHIFT`, never as lowercase-plus-SHIFT. So
///   the characters a terminal actually reports it with (`SHIFT_PROBE_CHARS`)
///   are swept as plain keys carrying exactly `SHIFT` — which is what exercises
///   the `SHIFT` branch of `trigger_matches_event(Char(_))` — and `SHIFT` rides
///   along with each chord rather than forming one of its own.
/// * On **non-`Char` codes** there is no character for it to form, so
///   `Shift-Left` is as much a chord as `Ctrl-Left` is, and `PROBE_CODES` gets a
///   `SHIFT`-only pass of its own. Without it `Shift-Left`/`Shift-Right` would
///   be swept by nothing: the other nine codes are covered by
///   `control_keys_ignore_modifiers` (whose `chord_modifier_sets` includes bare
///   `SHIFT`), but `Left` and `Right` are deliberately excluded from that
///   invariant because on them the modifier is load-bearing.
///
/// There is deliberately no allowlist here. If a chord fails this sweep, either
/// it is a real binding that needs a catalog entry, or the keymap arm it reached
/// is the defect.
fn key_universe() -> Vec<KeyEvent> {
    let mut events = Vec::new();
    for &ch in PROBE_CHARS {
        events.push(press(KeyCode::Char(ch)));
    }
    for &code in PROBE_CODES {
        events.push(press(code));
    }
    for &ch in SHIFT_PROBE_CHARS {
        events.push(modified_press(KeyCode::Char(ch), KeyModifiers::SHIFT));
    }
    // `SHIFT` alone over the non-`Char` codes. See the doc comment: on these
    // codes SHIFT forms no character, so it is a chord like any other, and this
    // is the only sweep that reaches `Shift-Left`/`Shift-Right`.
    for &code in PROBE_CODES {
        events.push(modified_press(code, KeyModifiers::SHIFT));
    }
    for base in [
        KeyModifiers::CONTROL,
        KeyModifiers::ALT,
        KeyModifiers::CONTROL | KeyModifiers::ALT,
        KeyModifiers::SUPER,
    ] {
        for modifiers in [base, base | KeyModifiers::SHIFT] {
            for &ch in PROBE_CHARS {
                events.push(modified_press(KeyCode::Char(ch), modifiers));
            }
            for &code in PROBE_CODES {
                events.push(modified_press(code, modifiers));
            }
        }
    }
    events
}

/// Concrete `KeyEvent`s that realize a given catalog `KeyTrigger`, used by
/// `no_orphan_catalog_bindings` to probe whether a binding's triggers
/// actually dispatch.
fn events_for_trigger(trigger: KeyTrigger) -> Vec<KeyEvent> {
    match trigger {
        // Both the bare and the SHIFT-carrying event, because a terminal may
        // report either for the same character. See `trigger_dispatches` for
        // why these two are folded with `all` rather than `any`.
        KeyTrigger::Char(c) => vec![
            press(KeyCode::Char(c)),
            modified_press(KeyCode::Char(c), KeyModifiers::SHIFT),
        ],
        KeyTrigger::CtrlChar(c) => vec![modified_press(KeyCode::Char(c), KeyModifiers::CONTROL)],
        KeyTrigger::Enter => vec![press(KeyCode::Enter)],
        KeyTrigger::Esc => vec![press(KeyCode::Esc)],
        KeyTrigger::Tab => vec![press(KeyCode::Tab)],
        KeyTrigger::BackTab => vec![press(KeyCode::BackTab)],
        KeyTrigger::Up => vec![press(KeyCode::Up)],
        KeyTrigger::Down => vec![press(KeyCode::Down)],
        KeyTrigger::Left => vec![press(KeyCode::Left)],
        KeyTrigger::Right => vec![press(KeyCode::Right)],
        KeyTrigger::AltLeft => vec![modified_press(KeyCode::Left, KeyModifiers::ALT)],
        KeyTrigger::AltRight => vec![modified_press(KeyCode::Right, KeyModifiers::ALT)],
        KeyTrigger::PageUp => vec![press(KeyCode::PageUp)],
        KeyTrigger::PageDown => vec![press(KeyCode::PageDown)],
        KeyTrigger::Backspace => vec![press(KeyCode::Backspace)],
        // Representative printable-text probes not otherwise bound in any
        // context we exercise.
        KeyTrigger::PrintableText => vec![press(KeyCode::Char('x')), press(KeyCode::Char('z'))],
    }
}

/// Whether a catalog trigger actually dispatches an action in `context`.
///
/// The fold over [`events_for_trigger`] differs by trigger, and the difference
/// is the whole point:
///
/// * `Char(c)` requires **every** probe to dispatch. Its two probes are the
///   same character as a terminal may report it — bare, or carrying `SHIFT` —
///   and `is_plain_text_key` accepts both, so a catalog entry for `c` claims
///   both. Folding them with `any` would let a half-dead arm (one whose guard
///   is `key.modifiers.is_empty()` instead of `is_plain_text_key`) pass on the
///   strength of the bare probe alone, which is exactly the defect the extra
///   probe exists to catch. An `any` fold makes an added probe strictly more
///   permissive; only `all` can make it catch anything.
/// * Every other trigger requires **some** probe to dispatch. `PrintableText`
///   in particular probes two representative characters to support a "some
///   printable character is accepted here" claim, not an "every character is"
///   one; the other triggers have a single probe, where the two folds agree.
fn trigger_dispatches(trigger: KeyTrigger, context: &nuvem::input::Context) -> bool {
    let events = events_for_trigger(trigger);
    match trigger {
        KeyTrigger::Char(_) => events
            .into_iter()
            .all(|ev| action_for_key(ev, context).is_some()),
        _ => events
            .into_iter()
            .any(|ev| action_for_key(ev, context).is_some()),
    }
}

/// A `Char(c)` trigger is *half-dead* in a context when the bare event
/// dispatches there but the SHIFT-carrying one does not — the arm behind it
/// guards with `key.modifiers.is_empty()` where every sibling arm uses
/// `is_plain_text_key`, so it honours half of what its catalog entry claims.
///
/// This is checked per trigger rather than folded into the "does this binding
/// fire at all" question, because that question is an `any` over the binding's
/// triggers: a binding listing both `Char('j')` and `Down` would be rescued by
/// `Down` and its half-dead `j` would go unreported. Returns `None` for every
/// non-`Char` trigger, and for a `Char` trigger that simply does not dispatch
/// in this context (that is the orphan check's business, not this one's).
fn half_dead_char_trigger(trigger: KeyTrigger, context: &nuvem::input::Context) -> Option<char> {
    let KeyTrigger::Char(c) = trigger else {
        return None;
    };
    let bare = action_for_key(press(KeyCode::Char(c)), context).is_some();
    let shifted = action_for_key(
        modified_press(KeyCode::Char(c), KeyModifiers::SHIFT),
        context,
    )
    .is_some();
    (bare && !shifted).then_some(c)
}

/// Whether a probed key event is "expected" to be a specific trigger match:
/// a concrete trigger (`Char`, `Enter`, ...) matches exactly that key event;
/// `PrintableText` matches any plain (empty or Shift-only modifiers) `Char`.
fn trigger_matches_event(trigger: KeyTrigger, ev: &KeyEvent) -> bool {
    match trigger {
        // SHIFT is character-forming, not a chord modifier — `is_plain_text_key`
        // in `src/input/keymap.rs` permits it, so the catalog side must too.
        // `SHIFT_PROBE_CHARS` is what reaches this branch from the forward sweep.
        KeyTrigger::Char(c) => {
            ev.code == KeyCode::Char(c)
                && (ev.modifiers.is_empty() || ev.modifiers == KeyModifiers::SHIFT)
        }
        KeyTrigger::CtrlChar(c) => {
            ev.code == KeyCode::Char(c) && ev.modifiers == KeyModifiers::CONTROL
        }
        KeyTrigger::Enter => ev.code == KeyCode::Enter,
        KeyTrigger::Esc => ev.code == KeyCode::Esc,
        KeyTrigger::Tab => ev.code == KeyCode::Tab,
        KeyTrigger::BackTab => ev.code == KeyCode::BackTab,
        KeyTrigger::Up => ev.code == KeyCode::Up,
        KeyTrigger::Down => ev.code == KeyCode::Down,
        KeyTrigger::Left => ev.code == KeyCode::Left && ev.modifiers.is_empty(),
        KeyTrigger::Right => ev.code == KeyCode::Right && ev.modifiers.is_empty(),
        KeyTrigger::AltLeft => ev.code == KeyCode::Left && ev.modifiers == KeyModifiers::ALT,
        KeyTrigger::AltRight => ev.code == KeyCode::Right && ev.modifiers == KeyModifiers::ALT,
        KeyTrigger::PageUp => ev.code == KeyCode::PageUp,
        KeyTrigger::PageDown => ev.code == KeyCode::PageDown,
        KeyTrigger::Backspace => ev.code == KeyCode::Backspace,
        KeyTrigger::PrintableText => {
            matches!(ev.code, KeyCode::Char(_))
                && (ev.modifiers.is_empty() || ev.modifiers == KeyModifiers::SHIFT)
        }
    }
}

/// A probed key is "covered by the catalog" iff some binding for the
/// context has a trigger that matches it, where a specific trigger
/// (`Char`/`Enter`/...) wins over a `PrintableText` trigger matching the
/// same key (mirrors keymap arm-ordering: specific arms are checked before
/// falling through to generic text-entry arms).
fn covered_by_catalog(ev: &KeyEvent, bindings: &[Binding]) -> bool {
    let specifically_matched = bindings.iter().any(|b| {
        b.triggers
            .iter()
            .any(|t| !matches!(t, KeyTrigger::PrintableText) && trigger_matches_event(*t, ev))
    });
    if specifically_matched {
        return true;
    }
    bindings.iter().any(|b| {
        b.triggers
            .iter()
            .any(|t| matches!(t, KeyTrigger::PrintableText) && trigger_matches_event(*t, ev))
    })
}

/// The one slice of the probe space no static catalog can express, exempted from
/// both parity sweeps by the two call sites below.
///
/// With `,` armed (`Mode::PluginPrefix`), `plugin_prefix_key` accepts *every*
/// plain-text character, and which of them names a plugin is the operator's
/// plugin file — runtime data a `&'static [KeyTrigger]` cannot hold. A catalog
/// entry claiming `j` opens a plugin would be false, and one enumerating a probe
/// alphabet would be a fixture pretending to be metadata.
///
/// Deliberately as narrow as the argument, and no narrower — three ways it
/// stays honest:
///
/// * It is an *event* predicate, never a context one. `Esc` in this mode is
///   perfectly knowable statically, and the orphan direction cannot cover for
///   it: that direction only rejects catalog entries which never dispatch, so it
///   can never notice a *missing* one. A context-level skip let
///   `PLUGIN_PREFIX_CANCEL` be deleted outright with the whole suite still
///   green — an armed mode with no advertised way out, and nothing objecting.
/// * It exempts only the *plain-text* characters, which is exactly the set the
///   keymap arm accepts. `Ctrl-C` is a `Char` event too, and it dispatches here
///   (`action_for_key` force-quits ahead of the mode match), so it stays swept
///   and `GLOBAL_FORCE_QUIT` stays required in this context's catalog.
/// * Nothing is exempted in any other context, and nothing at all is exempted
///   from the orphan direction.
///
/// `plugin_prefix_dispatches_every_character` stands in for what is exempted
/// here, and asserts strictly more than the sweeps would have.
fn catalog_cannot_express(ctx: HelpContext, ev: &KeyEvent) -> bool {
    ctx == HelpContext::PluginPrefix
        && matches!(ev.code, KeyCode::Char(_))
        && (ev.modifiers.is_empty() || ev.modifiers == KeyModifiers::SHIFT)
}

/// Guard test: proves the catalog is a faithful single source of truth for
/// every key `keymap::action_for_key` dispatches, in every `HelpContext`.
/// If someone adds a new keymap dispatch branch (or a new `HelpContext`)
/// without a corresponding catalog `Binding`, this test fails and names the
/// offending context + key.
#[test]
fn every_dispatched_key_has_catalog_metadata() {
    for &ctx in ALL_HELP_CONTEXTS {
        let context = context_for(ctx);
        let bindings = context_bindings(ctx);
        for ev in key_universe() {
            // The armed plugin prefix's plain characters are the one slice of
            // this sweep no static catalog can answer for. See
            // `catalog_cannot_express` for the argument and for why the exemption
            // is per event rather than per context — `Esc` and `Ctrl-C` in that
            // mode are still swept and still required to carry metadata.
            if catalog_cannot_express(ctx, &ev) {
                continue;
            }
            if action_for_key(ev, &context).is_some() {
                assert!(
                    covered_by_catalog(&ev, &bindings),
                    "{ctx:?}: key {ev:?} dispatches an action but has no catalog binding \
                     (add metadata to context_bindings({ctx:?}) in src/app/projection/binding.rs)"
                );
            }
        }
    }
}

/// Proves the mark/range/clear/unmark-visible keys (issue #35) are present
/// in the `ResourceList` catalog, keeping the footer/help text in sync with
/// the new keymap dispatch added for batch selection.
#[test]
fn resource_list_catalog_includes_mark_bindings() {
    let bindings = context_bindings(HelpContext::ResourceList);
    let keys: Vec<_> = bindings.iter().map(|b| b.key).collect();
    assert!(keys.contains(&"Space"), "missing Space (mark) binding");
    assert!(keys.contains(&"V"), "missing V (range mark) binding");
    assert!(keys.contains(&"\\"), "missing \\ (clear marks) binding");
    assert!(keys.contains(&"U"), "missing U (unmark visible) binding");
}

/// Proves the catalog never claims a key that the keymap doesn't actually
/// honor in that context ("no orphan entries").
#[test]
fn no_orphan_catalog_bindings() {
    for &ctx in ALL_HELP_CONTEXTS {
        let context = context_for(ctx);
        for binding in context_bindings(ctx) {
            let fires = binding
                .triggers
                .iter()
                .any(|trigger| trigger_dispatches(*trigger, &context));
            assert!(
                fires,
                "{ctx:?}: catalog binding {:?} (key {:?}) has no trigger that actually \
                 dispatches an action in this context",
                binding.label, binding.key
            );
            for trigger in binding.triggers {
                if let Some(c) = half_dead_char_trigger(*trigger, &context) {
                    panic!(
                        "{ctx:?}: catalog binding {:?} (key {:?}) claims {c:?}, but the keymap \
                         arm behind it dispatches the bare key and refuses the SHIFT-carrying \
                         one — guard it with `is_plain_text_key`, not `modifiers.is_empty()`",
                        binding.label, binding.key
                    );
                }
            }
        }
    }
}

/// Every modifier combination `key_universe` crosses its probes with, plus
/// `SHIFT` on its own — the full set of ways a terminal can decorate a key that
/// the app never asks it to decorate.
fn chord_modifier_sets() -> Vec<KeyModifiers> {
    let mut sets = vec![KeyModifiers::SHIFT];
    for base in [
        KeyModifiers::CONTROL,
        KeyModifiers::ALT,
        KeyModifiers::CONTROL | KeyModifiers::ALT,
        KeyModifiers::SUPER,
    ] {
        sets.push(base);
        sets.push(base | KeyModifiers::SHIFT);
    }
    sets
}

/// A comparable rendering of what a key dispatched.
///
/// Every action compares by value except one: `ConfirmTunnelLaunch` carries a
/// `started_at` that the keymap stamps with `Instant::now()` — the single impure
/// call in `keymap.rs` — so two dispatches of the same key differ in that field
/// alone and nothing else. The stamp is dropped here rather than compared, which
/// keeps the comparison free of any clock reading; the variant, and every field
/// of every other action, is still compared in full.
fn dispatch_signature(action: Option<Action>) -> String {
    match action {
        Some(Action::Resources(ResourcesAction::ConfirmTunnelLaunch { .. })) => {
            "Resources(ConfirmTunnelLaunch)".to_string()
        }
        other => format!("{other:?}"),
    }
}

/// The nine control keys whose keymap arms match on `key.code` alone, so they
/// dispatch the same action no matter what modifiers ride along.
const MODIFIER_INSENSITIVE_CODES: &[KeyCode] = &[
    KeyCode::Enter,
    KeyCode::Esc,
    KeyCode::Tab,
    KeyCode::BackTab,
    KeyCode::Up,
    KeyCode::Down,
    KeyCode::PageUp,
    KeyCode::PageDown,
    KeyCode::Backspace,
];

/// The control keys are **deliberately** lenient about modifiers, and this test
/// asserts that leniency rather than tolerating it.
///
/// Why the leniency is intentional: terminals speaking the kitty keyboard
/// protocol report stray modifiers on `Esc` and `Enter` — an `Esc` can arrive
/// carrying `SHIFT` or `ALT` that the operator never pressed. Guarding these
/// arms strictly, the way the character arms are guarded with
/// `is_plain_text_key`, would make the escape hatch *dead* in those terminals:
/// the help overlay, the confirmations and the previews would stop closing for
/// real users, and the only symptom would be a key that does nothing. So the
/// arms match on `key.code` alone, on purpose. A reader who arrives here from a
/// tightening spree is looking at a decision, not at sloppiness.
///
/// Why this is asserted rather than allowlisted, which is the whole point: an
/// allowlist would *excuse* the behaviour and guarantee the suite could never
/// fail on it again. This is the inverse. It pins invariance as a positive
/// property, so the day somebody adds a `Ctrl-Enter` binding without catalog
/// metadata, `Ctrl-Enter` stops agreeing with bare `Enter` and this test fails
/// naming the context and the key. That is precisely the case the Phase 3
/// plugin surface and the read-only / dangerous-key gating
/// need protected, and it is why this ticket exists.
///
/// `Left` and `Right` are deliberately absent from the list. On those two codes
/// the modifier IS load-bearing: `Alt-Left`/`Alt-Right` are the history keys
/// (handled in `normal_key` ahead of everything else) while plain `Left`/`Right`
/// are the `ResourceList` column cursor, so invariance there would be a bug, not
/// a policy. They are covered instead by the evidence file's paired
/// leak/binding assertions.
#[test]
fn control_keys_ignore_modifiers() {
    for &ctx in ALL_HELP_CONTEXTS {
        let context = context_for(ctx);
        for &code in MODIFIER_INSENSITIVE_CODES {
            let bare = dispatch_signature(action_for_key(press(code), &context));
            for modifiers in chord_modifier_sets() {
                let chorded =
                    dispatch_signature(action_for_key(modified_press(code, modifiers), &context));
                assert_eq!(
                    chorded, bare,
                    "{ctx:?}: {code:?} with {modifiers:?} dispatches {chorded:?} but bare \
                     {code:?} dispatches {bare:?} — the control keys are intentionally \
                     modifier-insensitive (terminals using the kitty keyboard protocol report \
                     stray modifiers on Esc/Enter, and guarding these arms strictly kills the \
                     escape hatch there). If this chord is a new deliberate binding, it needs \
                     catalog metadata in src/app/projection/binding.rs and this invariant has \
                     to be revisited on purpose, not relaxed"
                );
            }
        }
    }
}

/// Two blind spots the old three-chord probe list could not see:
/// `Alt-<letter>` and `Ctrl-<digit>`. The widened sweep came back clean on both
/// — every arm those chords could reach already guards with `is_plain_text_key`,
/// which rejects `ALT` and `CONTROL` outright. So this test pins a *verified
/// clean* property, not a guess: neither class dispatches anything, in any
/// `HelpContext`.
///
/// It is worth pinning because "clean" here rests entirely on a one-word guard
/// on each arm. Drop `if is_plain_text_key(key)` from a single `Char` arm and
/// that character's whole chord column starts dispatching — silently, because
/// the operator's `Alt-w` would look like a plain `w`.
///
/// The app's real Ctrl bindings do not collide with this: `Ctrl-C` (quit),
/// `Ctrl-T` (Tunnels) and `Ctrl-S` (Logging save) are all `Ctrl`-*letter*, so
/// they fall in neither class. There is deliberately no exemption list here —
/// if a combination in scope ever does dispatch, it is a real leak of exactly
/// the class this was opened for and the keymap arm behind it is the fix,
/// never a carve-out in this test.
#[test]
fn plain_text_arms_reject_chord_modifiers() {
    for &ctx in ALL_HELP_CONTEXTS {
        let context = context_for(ctx);
        for ch in ('a'..='z').chain('A'..='Z') {
            let ev = modified_press(KeyCode::Char(ch), KeyModifiers::ALT);
            assert_eq!(
                action_for_key(ev, &context),
                None,
                "{ctx:?}: Alt-{ch} dispatches an action. The app binds no Alt-<letter>, so the \
                 arm it reached is missing its `is_plain_text_key` guard — fix the arm in \
                 src/input/keymap.rs, do not exempt the key here"
            );
        }
        for ch in '0'..='9' {
            let ev = modified_press(KeyCode::Char(ch), KeyModifiers::CONTROL);
            assert_eq!(
                action_for_key(ev, &context),
                None,
                "{ctx:?}: Ctrl-{ch} dispatches an action. The app binds no Ctrl-<digit>, so the \
                 arm it reached is missing its `is_plain_text_key` guard — fix the arm in \
                 src/input/keymap.rs, do not exempt the key here"
            );
        }
    }
}

/// Override-aware analogue of [`covered_by_catalog`], over `DisplayBinding`s
/// (whose `triggers` are owned, and reflect the active overrides).
fn covered_by_display_catalog(ev: &KeyEvent, bindings: &[DisplayBinding]) -> bool {
    let specifically_matched = bindings.iter().any(|b| {
        b.triggers
            .iter()
            .any(|t| !matches!(t, KeyTrigger::PrintableText) && trigger_matches_event(*t, ev))
    });
    if specifically_matched {
        return true;
    }
    bindings.iter().any(|b| {
        b.triggers
            .iter()
            .any(|t| matches!(t, KeyTrigger::PrintableText) && trigger_matches_event(*t, ev))
    })
}

/// The override-aware parity guarantee: with a sample override set, the
/// override-aware catalog (`context_bindings_with`) and the override-aware
/// keymap dispatch agree in every `HelpContext`, in both directions. Mirrors
/// the two default-parity tests above but through the `_with` variants.
#[test]
fn parity_holds_under_overrides() {
    // Rebind four normal-mode actions to keys that free up their defaults.
    // `quit` and `help` are included specifically to guard the Help-context
    // twin case: `HELP_QUIT` is byte-identical to `GLOBAL_QUIT` but the Help
    // overlay quits via a hardcoded `q` (keymap::help_key), so a quit override
    // must NOT relabel it. If it were relabeled, this parity check fails for
    // `HelpContext::Help` in both directions ('q' dispatches but is uncovered,
    // and the relabeled 'x' is an orphan that never dispatches).
    //
    // `refresh` is deliberately rebound to a **Ctrl-char** rather than to a
    // plain letter: `override_trigger` in `src/input/keymap.rs` is the one place
    // a user-configured key becomes a `KeyTrigger`, and its Ctrl branch is the
    // only one that could widen a `Ctrl-<x>` binding into `Ctrl-Alt-<x>` and
    // `Ctrl-Shift-<x>`. With an all-plain sample this direction of the sweep
    // never reaches that branch, so the leak class the rest of the harness
    // exists to catch would be invisible in exactly the path the *user* controls.
    // `ctrl-r` is chosen because `ctrl-c`/`ctrl-s`/`ctrl-t` are reserved (see
    // `config::hotkeys::reserved_reason`) and would be rejected by validation.
    let overrides = KeyOverrides::from_file(&raw(&[
        ("home", "H"),
        ("refresh", "ctrl-r"),
        ("quit", "x"),
        ("help", "z"),
    ]))
    .expect("sample overrides should be valid");

    for &ctx in ALL_HELP_CONTEXTS {
        let mut context = context_for(ctx);
        context.hotkeys = overrides.clone();
        let bindings = context_bindings_with(ctx, &overrides);

        // Direction 1: every dispatched key has override-aware catalog metadata.
        for ev in key_universe() {
            // Exempted here for the same reason, and only the same reason, as in
            // `every_dispatched_key_has_catalog_metadata`, and through the same
            // event predicate rather than a context skip. Overrides change
            // nothing about the question: `plugin_prefix_key` reads no hotkey
            // override at all, so this direction asks the identical unanswerable
            // one of the identical events.
            if catalog_cannot_express(ctx, &ev) {
                continue;
            }
            if action_for_key(ev, &context).is_some() {
                assert!(
                    covered_by_display_catalog(&ev, &bindings),
                    "{ctx:?} (under overrides): key {ev:?} dispatches but has no \
                     override-aware catalog binding"
                );
            }
        }

        // Direction 2: no orphan override-aware catalog bindings.
        for binding in &bindings {
            let fires = binding
                .triggers
                .iter()
                .any(|trigger| trigger_dispatches(*trigger, &context));
            assert!(
                fires,
                "{ctx:?} (under overrides): catalog binding {:?} (key {:?}) has no \
                 trigger that dispatches in this context",
                binding.label, binding.key
            );
            for trigger in &binding.triggers {
                if let Some(c) = half_dead_char_trigger(*trigger, &context) {
                    panic!(
                        "{ctx:?} (under overrides): catalog binding {:?} (key {:?}) claims \
                         {c:?}, but the keymap arm behind it dispatches the bare key and \
                         refuses the SHIFT-carrying one",
                        binding.label, binding.key
                    );
                }
            }
        }
    }
}

/// What `every_dispatched_key_has_catalog_metadata` skips for
/// `HelpContext::PluginPrefix`, asserted directly: every character reaches the
/// reducer as *itself*, `Esc` cancels, and nothing else in the sweep
/// dispatches at all.
///
/// Stronger than the property it stands in for, which only asks whether a key
/// dispatches *something*. A `plugin_prefix_key` that mapped every press to
/// one wrong character would satisfy that property and fail this.
///
/// The chord half of the sweep is asserted too, because the exemption would
/// otherwise drop it: bare `SHIFT` is character-forming and so still dispatches
/// (as the shifted face, over `SHIFT_PROBE_CHARS` — the characters a terminal
/// actually reports it with), every true chord is refused by `is_plain_text_key`,
/// `Ctrl-C` force-quits, and `Esc` cancels under any decoration because its arm
/// matches on `key.code` alone. Dropping that guard from the `Char` arm would
/// leak the operator's `Alt-w` into a plugin key, and this is the assertion that
/// would notice.
#[test]
fn plugin_prefix_dispatches_every_character() {
    let context = context_for(HelpContext::PluginPrefix);
    let pressed = |ch: char| Some(Action::Resources(ResourcesAction::PluginPrefixPress(ch)));
    let cancelled = || Some(Action::Resources(ResourcesAction::CancelPluginPrefix));

    for &ch in PROBE_CHARS {
        assert_eq!(
            action_for_key(press(KeyCode::Char(ch)), &context),
            pressed(ch),
            "{ch:?} must reach the reducer as itself"
        );
    }
    assert_eq!(action_for_key(press(KeyCode::Esc), &context), cancelled());
    for &code in PROBE_CODES {
        if code == KeyCode::Esc {
            continue;
        }
        assert_eq!(
            action_for_key(press(code), &context),
            None,
            "{code:?} must be swallowed, not fall through to the list bindings"
        );
    }

    // Bare `SHIFT` is swept over `SHIFT_PROBE_CHARS` only, matching this file's
    // model of the terminal (see `key_universe`): on a character, SHIFT is
    // character-forming rather than a chord, so Shift+j arrives as `Char('J')`
    // carrying SHIFT and lowercase-plus-SHIFT is an event no terminal produces.
    // Asserting a dispatch for it would pin a keypress that cannot happen, and
    // would then block a legitimate future tightening of `is_plain_text_key`
    // with a failure about an impossible event.
    for &ch in SHIFT_PROBE_CHARS {
        assert_eq!(
            action_for_key(
                modified_press(KeyCode::Char(ch), KeyModifiers::SHIFT),
                &context
            ),
            pressed(ch),
            "the shifted face {ch:?} must reach the reducer as itself"
        );
    }

    // The true chords over the characters: every set except bare `SHIFT`, which
    // is not a chord on this half and was swept just above.
    for modifiers in chord_modifier_sets() {
        if modifiers == KeyModifiers::SHIFT {
            continue;
        }
        for &ch in PROBE_CHARS {
            // The two globals resolve ahead of the mode match, which is why
            // `GLOBAL_FORCE_QUIT` and `GLOBAL_CANCEL_PLUGIN` are both in this
            // context's catalog — and, since `catalog_cannot_express` does not
            // exempt them, why the parity sweeps require those entries rather
            // than merely tolerating them. `Ctrl-x` joined `Ctrl-C` up there
            // later: an armed prefix renders no modal, so a cancel key that
            // died here died invisibly.
            let expected = if modifiers == KeyModifiers::CONTROL && ch == 'c' {
                Some(Action::QuitRequested)
            } else if modifiers == KeyModifiers::CONTROL && ch == 'x' {
                Some(Action::System(SystemAction::CancelRunningPlugins))
            } else {
                None
            };
            assert_eq!(
                action_for_key(modified_press(KeyCode::Char(ch), modifiers), &context),
                expected,
                "{ch:?} with {modifiers:?} in the armed prefix"
            );
        }
    }

    // The chords over the non-`Char` codes, bare `SHIFT` included: on these there
    // is no character for it to form, so `Shift-Esc` is as much a chord as
    // `Ctrl-Esc`. `Esc` cancels under all of them because its arm matches
    // `key.code` alone; nothing else here dispatches at all.
    for modifiers in chord_modifier_sets() {
        for &code in PROBE_CODES {
            let expected = if code == KeyCode::Esc {
                cancelled()
            } else {
                None
            };
            assert_eq!(
                action_for_key(modified_press(code, modifiers), &context),
                expected,
                "{code:?} with {modifiers:?} in the armed prefix"
            );
        }
    }
}

/// With `home` rebound to `H`, the override-aware help model teaches `H` for
/// Home (never the default `h`), proving footer/help show rebound keys.
#[test]
fn footer_and_help_show_rebound_keys() {
    let overrides =
        KeyOverrides::from_file(&raw(&[("home", "H")])).expect("override should be valid");
    let model = help_model_with(&RootState::default(), &overrides);

    let navigation = model
        .sections
        .iter()
        .find(|section| section.title == "Navigation")
        .expect("help model has a Navigation section");

    assert!(
        navigation
            .bindings
            .iter()
            .any(|binding| binding.key == "H" && binding.label == "Home"),
        "Navigation section should list H for Home under the override"
    );
    assert!(
        !navigation.bindings.iter().any(|binding| binding.key == "h"),
        "Navigation section should not still list the default h for Home"
    );
}

/// `y` means two different things in the two resource contexts, and the help
/// overlay has to say so: in the list it yanks the Cloud Storage `gs://` URI,
/// while in a detail pane it yanks the selected field — or the whole document
/// on a JSON tab — for every service.
#[test]
fn the_detail_context_describes_y_as_a_field_copy_not_a_gs_uri() {
    let label_for_y = |ctx: HelpContext| -> &'static str {
        context_bindings(ctx)
            .into_iter()
            .find(|binding| binding.key == "y")
            .unwrap_or_else(|| panic!("{ctx:?} should list a y binding"))
            .label
    };

    assert_eq!(
        label_for_y(HelpContext::ResourceDetail),
        "Copy field / document"
    );
    assert_eq!(label_for_y(HelpContext::ResourceList), "Copy gs:// URI");
}
