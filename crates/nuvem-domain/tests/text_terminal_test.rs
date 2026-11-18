//! Pins the display sanitizer's control classes and the string-sequence
//! terminators the escape stripper recognises.

use nuvem_domain::gcp::text::{
    rendered_text_is_clean, sanitize_clipboard_text, sanitize_display_text, sanitize_rendered_text,
};

/// Each control class is replaced on its own: C0, DEL, and C1 — a conjunction
/// of the three tests would let one class carry the others.
#[test]
fn each_control_class_is_replaced_alone() {
    assert_eq!(sanitize_display_text("\u{01}"), "\u{FFFD}");
    assert_eq!(sanitize_display_text("\u{7f}"), "\u{FFFD}");
    assert_eq!(sanitize_display_text("\u{80}"), "\u{FFFD}");
    assert_eq!(sanitize_display_text("\u{9f}"), "\u{FFFD}");
    assert_eq!(sanitize_display_text("naïve 🚀 fine"), "naïve 🚀 fine");
}

/// An OSC body terminated by `ESC \` (the two-byte ST) ends the sequence
/// there: the terminator is consumed with the body, and text after it
/// survives. BEL termination is pinned by the existing escape tests; this one
/// exists because the ESC arm of the terminator match had no test of its own.
#[test]
fn string_sequence_terminated_by_esc_backslash_ends_there() {
    assert_eq!(sanitize_rendered_text("a\u{1b}]0;title\u{1b}\\b"), "ab");
}

// --- Characters that reorder or hide text (FIL-audit) ---

/// The spoof this policy exists for: `U+202E` reverses everything after it, so
/// a row an operator reads as one resource is another one. On a rendered
/// surface the character is replaced rather than removed, because a name that
/// was tampered with should look tampered with — removing it silently would
/// leave a plausible name behind, which is what the attacker wanted.
#[test]
fn a_right_to_left_override_is_marked_on_a_rendered_surface() {
    let rendered = sanitize_rendered_text("web\u{202E}gnp.txt");

    assert!(
        !rendered.contains('\u{202E}'),
        "the override must not survive: {rendered:?}"
    );
    assert!(
        rendered.contains('\u{FFFD}'),
        "and its absence must be visible: {rendered:?}"
    );
}

/// On the clipboard the same character is dropped instead. A `U+FFFD` pasted
/// into a shell is a character the command did not have; the value has to
/// still work.
#[test]
fn a_right_to_left_override_is_removed_from_the_clipboard() {
    assert_eq!(sanitize_clipboard_text("web\u{202E}gnp.txt"), "webgnp.txt");
}

/// Zero-width characters hide rather than reorder: two names differing only by
/// one look identical in a list and are different resources.
#[test]
fn zero_width_separators_do_not_survive_to_either_sink() {
    assert_eq!(sanitize_clipboard_text("web\u{200B}-1"), "web-1");
    assert_eq!(sanitize_clipboard_text("web\u{FEFF}-1"), "web-1");
    assert!(!sanitize_rendered_text("web\u{2060}-1").contains('\u{2060}'));
}

/// The bidi isolates are the newer spelling of the same trick and go with the
/// overrides.
#[test]
fn bidi_isolates_are_handled_like_the_overrides() {
    for isolate in ['\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}', '\u{061C}'] {
        let value = format!("a{isolate}b");
        assert_eq!(
            sanitize_clipboard_text(&value),
            "ab",
            "{isolate:?} must not reach the clipboard"
        );
    }
}

/// The exclusions, which matter as much as the inclusions: ZWJ and ZWNJ are
/// the same Unicode category and are ordinary text. ZWJ is what holds a
/// multi-person emoji together, and both are needed to spell Devanagari and
/// Persian. A whole-category sweep would corrupt real content.
#[test]
fn joiners_are_ordinary_text_and_are_left_alone() {
    let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";

    assert_eq!(sanitize_clipboard_text(family), family);
    assert_eq!(sanitize_display_text(family), family);
    assert_eq!(sanitize_clipboard_text("\u{200C}"), "\u{200C}");
}

/// And ordinary names are untouched, so the policy costs nothing in the case
/// that is every real case.
#[test]
fn an_ordinary_resource_name_is_unchanged_by_the_new_rules() {
    assert_eq!(sanitize_clipboard_text("web-server-1"), "web-server-1");
    assert_eq!(sanitize_rendered_text("web-server-1"), "web-server-1");
}

// --- The fast path and the policy must agree (FIL-audit) ---

proptest::proptest! {
    /// The claim `rendered_text_is_clean` makes, held over generated input
    /// rather than over examples chosen alongside it: whenever it says a value
    /// needs nothing done to it, doing the work must produce that same value.
    ///
    /// Asserted this way round on purpose. A test that only checked
    /// "sanitising twice is stable", or that compared the predicate to itself,
    /// would pass with both halves wrong together; this one fails the moment
    /// the predicate admits a character either step would have touched, which
    /// is the drift that a fast path invites.
    ///
    /// `\PC` is excluded from the generator only to keep the *interesting*
    /// half dense — the escape and format characters are added back
    /// explicitly by the alternation, so the clean and dirty cases both occur.
    #[test]
    fn sanitize_rendered_text_is_identity_on_clean_text(
        value in proptest::string::string_regex("[a-z0-9 \\-_./:\u{1b}\u{7f}\u{202e}\u{200b}\u{200d}\u{9b}]{0,40}")
            .expect("a valid generator")
    ) {
        if rendered_text_is_clean(&value) {
            proptest::prop_assert_eq!(
                sanitize_rendered_text(&value),
                value.clone(),
                "the predicate called this clean, but sanitising changed it"
            );
        } else {
            proptest::prop_assert_ne!(
                sanitize_rendered_text(&value),
                value.clone(),
                "the predicate called this dirty, but sanitising left it alone"
            );
        }
    }
}
