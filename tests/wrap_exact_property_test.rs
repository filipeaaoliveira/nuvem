//! `wrap_exact` never alters the text it wraps.
//!
//! The confirmation modals carry the exact argv they are about to run. Wrapping
//! them made the modal fit an ordinary terminal, and it must not have made the
//! command read differently: a wrap that collapsed two spaces inside an argument
//! would misrepresent the command in the one surface whose whole job is showing
//! it truthfully.
//!
//! A property rather than examples, because "never alters" is a claim about
//! every input and the interesting ones are the ones nobody thinks to write:
//! runs of spaces, a token longer than the width, a trailing space, a wide
//! glyph on the boundary.

use nuvem::ui::shared::components::wrap_exact;
use proptest::prelude::*;
use unicode_width::UnicodeWidthStr;

proptest! {
    /// Splitting only: the pieces reassemble into exactly what went in.
    #[test]
    fn wrap_exact_never_alters_the_text(text in ".{0,200}", width in 1usize..=40) {
        prop_assert_eq!(wrap_exact(&text, width).concat(), text);
    }

    /// And it actually wraps: no line is wider than the width it was given,
    /// unless it is a single glyph that cannot be narrower.
    #[test]
    fn no_wrapped_line_is_wider_than_the_width(text in ".{0,200}", width in 1usize..=40) {
        for line in wrap_exact(&text, width) {
            prop_assert!(
                UnicodeWidthStr::width(line.as_str()) <= width || line.chars().count() == 1,
                "line {line:?} is wider than {width}"
            );
        }
    }

    /// Minimal: no two adjacent lines could have been one.
    ///
    /// The wrap must use as few lines as the width allows, and this is what says
    /// so — if joining a line with the one after it would still fit, the split
    /// was unnecessary. The mutation gate is what asked for it: `replace > with
    /// >=` on the loop condition survived every other property here, because an
    /// extra split preserves the text, stays inside the width, and terminates.
    /// What it changes is only that a chunk exactly filling the width gets
    /// broken at its last space when it did not have to be.
    ///
    /// Generated from printable ASCII rather than from arbitrary Unicode, and
    /// that restriction is load-bearing rather than convenient: this property
    /// asks whether two lines "would have fit together", which presupposes that
    /// `UnicodeWidthStr::width` is additive — and it is not. A run of
    /// noncharacters and zero-width codepoints can make `width(a + b)` smaller
    /// than `width(a) + width(b)`, so the concatenation fits even though each
    /// split was forced by the measure the algorithm uses. That is a property of
    /// the measure, not a defect in the wrap, and it is what a `.{0,200}`
    /// generator found.
    ///
    /// The other three properties here keep the unrestricted generator, because
    /// they hold for every input. This is the one that cannot.
    #[test]
    fn the_wrap_is_minimal(text in "[ -~]{0,200}", width in 1usize..=40) {
        let lines = wrap_exact(&text, width);
        for pair in lines.windows(2) {
            let joined = format!("{}{}", pair[0], pair[1]);
            prop_assert!(
                UnicodeWidthStr::width(joined.as_str()) > width,
                "{:?} and {:?} fit in {width} columns together and should not \
                 have been split",
                pair[0],
                pair[1]
            );
        }
    }

    /// Progress: a non-empty input always produces at least one line, and the
    /// loop terminates — a version that could split at offset zero would hang
    /// rather than fail, so this is what catches it.
    #[test]
    fn wrapping_always_terminates_and_produces_lines(text in ".{1,200}", width in 1usize..=40) {
        prop_assert!(!wrap_exact(&text, width).is_empty());
    }
}

/// The case the modals actually hit.
#[test]
fn a_long_command_wraps_at_a_space_rather_than_mid_token() {
    let command = "Command: gcloud --configuration dev compute instances stop api-dev-1 --zone europe-west1-b";
    let lines = wrap_exact(command, 62);
    assert!(lines.len() > 1, "this command must wrap at 62 columns");
    assert_eq!(lines.concat(), command);
    assert!(
        lines[0].ends_with(' '),
        "the break should fall after a space, keeping it on the first line: {:?}",
        lines[0]
    );
}

/// A token with no space in it has nowhere to break and is split rather than
/// overflowing the pane.
#[test]
fn a_token_longer_than_the_width_is_split() {
    let lines = wrap_exact(&"x".repeat(100), 30);
    assert_eq!(lines.len(), 4);
    assert_eq!(lines.concat(), "x".repeat(100));
}
