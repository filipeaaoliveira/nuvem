//! Making untrusted text safe to render, to wrap, or to put on the clipboard.
//!
//! Resource names, labels and metadata are attacker-influenced and arrive
//! unfiltered. An escape sequence in one repaints the terminal; a bidi override
//! makes a name draw as something other than what it is. The three policies
//! here are deliberately separate rather than one shared helper, because the
//! right answer differs by sink: the replacement character that shows tampering
//! on screen would be a character the command did not have once pasted into a
//! shell.

/// Whether `c` changes how the text around it is ordered, or hides text
/// outright, without occupying a column of its own.
///
/// These are the characters that make a name render as something other than
/// what it is. `U+202E RIGHT-TO-LEFT OVERRIDE` is the pointed example: dropped
/// into a resource name, everything after it draws in reverse, so an operator
/// reading a row — or pasting it into a shell — acts on a string they never
/// saw. The zero-width members hide rather than reorder: two names differing
/// only by a `U+200B` look identical in a list and are different resources.
///
/// **Not every `Cf` character.** `U+200D ZERO WIDTH JOINER` and `U+200C ZERO
/// WIDTH NON-JOINER` share the category and are ordinary text: ZWJ is what
/// holds a multi-person emoji together, and both are required to spell
/// Devanagari and Persian correctly. Sweeping the whole category would
/// silently corrupt legitimate content to defend against a class those two are
/// not part of, so the set is named rather than derived.
fn reorders_or_hides_text(c: char) -> bool {
    matches!(
        c,
        // Bidirectional overrides, embeddings and isolates.
        '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'
        // Zero-width characters that separate or join without being seen.
        | '\u{200B}' | '\u{2060}' | '\u{FEFF}'
    )
}

/// Whether [`sanitize_rendered_text`] would leave `value` exactly as it is.
///
/// Exists so a hot path can skip the work rather than skip the policy. The
/// table projection sanitises every cell it builds — up to a few thousand per
/// frame — and almost every one is an ordinary GCP name that no rule touches;
/// answering "nothing here needs changing" without allocating is what makes
/// sanitising by construction affordable there.
///
/// It is the exact complement of what the two steps act on, not a second
/// opinion about them: `strip_terminal_escapes` only ever removes sequences
/// introduced by `ESC` or a C1 byte, and [`sanitize_display_text`] only ever
/// replaces those same controls and the characters
/// `reorders_or_hides_text` names. With none of them present both steps are
/// the identity, which `sanitize_rendered_text_is_identity_on_clean_text` in
/// the domain tests holds it to over arbitrary generated input.
pub fn rendered_text_is_clean(value: &str) -> bool {
    !value.chars().any(|c| {
        let code = c as u32;
        code < 0x20 || code == 0x7f || (0x80..=0x9f).contains(&code) || reorders_or_hides_text(c)
    })
}

/// Replaces terminal-unsafe characters with the Unicode replacement character
/// so hostile resource names cannot corrupt the TUI. C0 (< 0x20), DEL (0x7f),
/// C1 (0x80..=0x9f), and the reordering/hiding characters
/// `reorders_or_hides_text` names are replaced; all other characters
/// (including normal Unicode and emoji) are preserved unchanged.
///
/// Replaced rather than removed, here and only here: this is the *rendered*
/// policy, and a name that was tampered with should look tampered with. A
/// removed character leaves a plausible name behind, which is the outcome an
/// attacker wanted; `U+FFFD` occupies a column and says something was here.
pub fn sanitize_display_text(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            let code = c as u32;
            let unsafe_control = code < 0x20 || code == 0x7f || (0x80..=0x9f).contains(&code);
            if unsafe_control || reorders_or_hides_text(c) {
                '\u{FFFD}'
            } else {
                c
            }
        })
        .collect()
}

/// Sanitizes a value bound for the renderer: one buffer line, whether that is
/// a detail-pane row, the status line, or a key hint. Whole terminal escape
/// sequences are removed (leaving no readable `[31m` residue behind) and every
/// remaining control character is replaced as in [`sanitize_display_text`],
/// newlines and tabs included — a single line has no use for either.
///
/// Deliberately *not* the same policy as [`sanitize_clipboard_text`]: the two
/// sinks differ, and collapsing them into one would either leak escapes onto a
/// buffer cell or silently flatten a multi-line value on the clipboard.
pub fn sanitize_rendered_text(value: &str) -> String {
    sanitize_display_text(&strip_terminal_escapes(value))
}

/// Sanitizes a value bound for a rendered *paragraph*: text a surface will wrap
/// over several buffer lines, such as the help overlay's config-problem list.
///
/// The same policy as [`sanitize_rendered_text`] in what it removes, and it
/// differs in one thing only: whitespace becomes a space rather than a
/// replacement character. A wrapper breaks on whitespace, so a newline has to
/// stay a break point — turned into `U+FFFD` it would instead fuse the words on
/// either side into one unbreakable token.
///
/// The order is load-bearing. Escapes are stripped *first*, before the
/// whitespace collapse: collapsing first turns the introducing `ESC` into a
/// space, after which the sequence is no longer recognisable and `[31m`
/// survives as readable text. That is also why this lives here rather than as a
/// collapse bolted onto [`sanitize_rendered_text`] at a call site — the two
/// steps are one policy, and splitting them is exactly the bug.
pub fn sanitize_rendered_paragraph(value: &str) -> String {
    let stripped = strip_terminal_escapes(value);
    sanitize_display_text(&stripped.replace(|c: char| c.is_whitespace() || c.is_control(), " "))
}

/// Sanitizes a value bound for the system clipboard. Terminal escape sequences
/// are removed — no configuration value legitimately carries one, and the
/// clipboard is pasted straight back into a terminal — as are the remaining
/// control characters, *except* newlines and tabs: multi-line values such as a
/// `startup-script` or an `ssh-keys` metadata entry are ordinary, and an
/// operator yanking one expects it whole. Carriage returns are dropped, so a
/// CRLF value arrives newline-separated.
///
/// See [`sanitize_rendered_text`] for the sibling policy the rendered surfaces
/// use, and why the two must stay apart.
pub fn sanitize_clipboard_text(value: &str) -> String {
    strip_terminal_escapes(value)
        .chars()
        .filter(|c| {
            let code = *c as u32;
            if reorders_or_hides_text(*c) {
                // Dropped rather than replaced, unlike the rendered policy: a
                // `U+FFFD` pasted into a shell is a character the command did
                // not have, and the clipboard's job is to carry a value that
                // still works. Removing is what makes the pasted string mean
                // what the operator read.
                return false;
            }
            *c == '\n'
                || *c == '\t'
                || !(code < 0x20 || code == 0x7f || (0x80..=0x9f).contains(&code))
        })
        .collect()
}

/// Removes ANSI/terminal escape sequences, in both their `ESC`-introduced and
/// their C1 single-character forms: CSI sequences (`ESC [` … final byte) and
/// the string-terminated families OSC/DCS/SOS/PM/APC (`ESC ]` … `BEL` or `ST`),
/// plus the two-character `ESC <char>` forms. Characters that are not part of a
/// sequence — including lone control characters — are left for the caller's
/// own policy to handle.
///
/// Deliberately private: it is the shared first step of the three public
/// policies above, not a primitive for callers to assemble a fourth from.
pub(crate) fn strip_terminal_escapes(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\u{1b}' => match chars.peek().copied() {
                Some('[') => {
                    chars.next();
                    consume_control_sequence(&mut chars);
                }
                Some(']' | 'P' | 'X' | '^' | '_') => {
                    chars.next();
                    consume_string_sequence(&mut chars);
                }
                // Two-character forms (`ESC c`, `ESC 7`, ...). A following
                // control character is not part of the sequence and is left in
                // place for the caller's policy.
                Some(next) if ('\u{20}'..='\u{7e}').contains(&next) => {
                    chars.next();
                }
                _ => {}
            },
            // C1 CSI.
            '\u{9b}' => consume_control_sequence(&mut chars),
            // C1 DCS/SOS/OSC/PM/APC.
            '\u{90}' | '\u{98}' | '\u{9d}' | '\u{9e}' | '\u{9f}' => {
                consume_string_sequence(&mut chars)
            }
            _ => out.push(c),
        }
    }
    out
}

/// Consumes a CSI body: parameter and intermediate bytes (`0x20..=0x3f`)
/// followed by one final byte (`0x40..=0x7e`). A truncated sequence simply ends
/// with the input.
fn consume_control_sequence(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    while let Some(next) = chars.peek().copied() {
        if ('\u{20}'..='\u{3f}').contains(&next) {
            chars.next();
            continue;
        }
        if ('\u{40}'..='\u{7e}').contains(&next) {
            chars.next();
        }
        return;
    }
}

/// Consumes a string sequence body up to and including its terminator: `BEL`,
/// `ST` (`ESC \`), or the C1 `ST` (`0x9c`).
fn consume_string_sequence(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    while let Some(next) = chars.next() {
        match next {
            '\u{07}' | '\u{9c}' => return,
            '\u{1b}' => {
                if chars.peek() == Some(&'\\') {
                    chars.next();
                }
                return;
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The paragraph policy exists for wrapped surfaces, so whitespace has to
    /// survive as a break point rather than become a glyph: `sanitize_rendered_text`
    /// would map the newline to U+FFFD and fuse the two words together.
    #[test]
    fn paragraph_text_collapses_whitespace_to_break_points() {
        assert_eq!(
            sanitize_rendered_paragraph("short\nproblem\twith  gaps"),
            "short problem with  gaps"
        );
        assert_eq!(
            sanitize_rendered_text("short\nproblem"),
            "short\u{fffd}problem"
        );
    }

    /// Escapes must be stripped *before* whitespace is collapsed: collapsing
    /// first turns the introducing ESC into a space, after which the sequence
    /// is unrecognisable and `[31m` survives as readable text.
    #[test]
    fn paragraph_text_removes_whole_escape_sequences() {
        assert_eq!(
            sanitize_rendered_paragraph("a\u{1b}[31mb\u{1b}]0;title\u{07}c\u{1b}7d"),
            "abcd"
        );
        assert_eq!(sanitize_rendered_paragraph("a\u{9b}31mb"), "ab");
    }

    /// Lone control characters become break points rather than glyphs: DEL and
    /// the C1 range are `char::is_control()`, so the collapse claims them and
    /// the display sanitizer that follows is a backstop, not the first line of
    /// defence. Either way nothing controlling reaches a buffer cell.
    #[test]
    fn paragraph_text_leaves_no_control_character_behind() {
        assert_eq!(sanitize_rendered_paragraph("a\u{7f}b\u{0}c"), "a b c");
        assert!(
            !sanitize_rendered_paragraph("a\u{7f}b\u{0}c")
                .chars()
                .any(char::is_control)
        );
    }

    #[test]
    fn clipboard_text_keeps_newlines_and_tabs() {
        assert_eq!(
            sanitize_clipboard_text("#!/bin/sh\n\techo hello\n"),
            "#!/bin/sh\n\techo hello\n"
        );
    }

    #[test]
    fn clipboard_text_removes_whole_escape_sequences() {
        // CSI colour, CSI erase, an OSC title with its BEL terminator, and a
        // two-character ESC form: every one leaves nothing behind, not even
        // the readable `[31m` residue a control-character replacement leaves.
        assert_eq!(
            sanitize_clipboard_text("a\u{1b}[31mb\u{1b}[2Jc\u{1b}]0;title\u{07}d\u{1b}7e"),
            "abcde"
        );
        // C1 forms of the same sequences.
        assert_eq!(
            sanitize_clipboard_text("a\u{9b}31mb\u{9d}0;t\u{9c}c"),
            "abc"
        );
    }

    #[test]
    fn clipboard_text_drops_remaining_control_characters() {
        assert_eq!(sanitize_clipboard_text("a\u{0}b\rc\u{7f}d"), "abcd");
    }

    #[test]
    fn clipboard_text_preserves_ordinary_unicode() {
        assert_eq!(sanitize_clipboard_text("café-☕-测试"), "café-☕-测试");
    }

    #[test]
    fn rendered_text_strips_sequences_and_replaces_every_control_character() {
        assert_eq!(
            sanitize_rendered_text("boot\u{1b}[2Jed\nkey\ttab"),
            "booted\u{FFFD}key\u{FFFD}tab"
        );
    }

    #[test]
    fn the_rendered_and_clipboard_policies_differ_on_newlines_by_design() {
        let hostile = "line one\n\u{1b}[31mline two";

        // One buffer line: the newline cannot survive it.
        assert_eq!(sanitize_rendered_text(hostile), "line one\u{FFFD}line two");
        // The clipboard has no such constraint, and a multi-line value pasted
        // back has to still be multi-line.
        assert_eq!(sanitize_clipboard_text(hostile), "line one\nline two");
    }

    #[test]
    fn a_truncated_escape_sequence_does_not_swallow_the_rest_of_a_line() {
        // A bare ESC before a control character is not a two-character form:
        // the ESC goes, the newline stays.
        assert_eq!(sanitize_clipboard_text("a\u{1b}\nb"), "a\nb");
        // A CSI with no final byte runs to the end of the input, as a terminal
        // would treat it.
        assert_eq!(sanitize_clipboard_text("a\u{1b}[31"), "a");
    }
}
