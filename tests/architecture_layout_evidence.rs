//! Evidence generator — what an author sees when `ARCHITECTURE.md`
//! and the source tree disagree.
//!
//! The value of the drift check is not that it fails; it is that the failure
//! tells an author what drifted, in which direction, and where to go and fix
//! it. That message is only ever printed when the repository is broken, so it
//! cannot be shown from a passing suite and it cannot be shown by breaking the
//! real document.
//!
//! So this file drives the same parse/compare/format functions the repository
//! test uses (`support/architecture_layout.rs`) against a **synthetic**
//! document and a **synthetic** file set held entirely in memory. Nothing here
//! reads or writes `ARCHITECTURE.md`, walks the source tree, or depends on the
//! working directory. The assertions always run; the transcript is written to
//! the evidence directory only when `NUVEM_EVIDENCE_DIR` is set.

use std::fs;
use std::path::PathBuf;

use crate::support::architecture_layout as layout;

use layout::{
    ARCHITECTURE_DOC, BEGIN_MARKER, END_MARKER, SECTION_HEADING, compare_listings, parse_tree,
    verified_block,
};

/// A stand-in for `ARCHITECTURE.md`, shaped like the real one — prose, the two
/// markers, one labelled ```text fence — and small enough to read whole.
///
/// Its listing has drifted in both directions at once: `src/app/reducer/tunnels.rs`
/// was added to the tree without being listed, and `src/ui/legacy.rs` was
/// deleted without being unlisted.
fn drifted_document() -> String {
    format!(
        "# Synthetic Architecture\n\
         \n\
         {SECTION_HEADING}\n\
         \n\
         The tree below is checked in both directions.\n\
         \n\
         {BEGIN_MARKER}\n\
         \n\
         ```text\n\
         # VERIFIED - checked in both directions against the filesystem\n\
         src/\n  \
         app/\n    \
         mod.rs\n    \
         reducer/\n      \
         mod.rs\n  \
         main.rs\n  \
         ui/\n    \
         legacy.rs\n    \
         mod.rs\n\
         ```\n\
         \n\
         {END_MARKER}\n"
    )
}

/// The synthetic tree "on disk": the same modules, except `reducer/tunnels.rs`
/// exists and `ui/legacy.rs` does not.
fn synthetic_source_tree() -> Vec<String> {
    [
        "src/app/mod.rs",
        "src/app/reducer/mod.rs",
        "src/app/reducer/tunnels.rs",
        "src/main.rs",
        "src/ui/mod.rs",
    ]
    .iter()
    .map(|path| (*path).to_string())
    .collect()
}

fn evidence_dir() -> Option<PathBuf> {
    let dir = std::env::var("NUVEM_EVIDENCE_DIR").ok()?;
    let path = PathBuf::from(dir);
    fs::create_dir_all(&path).ok()?;
    Some(path)
}

/// Writes a captured transcript, boxed so the message's own line breaks and
/// indentation are visible as the author would see them in CI output.
fn write_transcript(name: &str, caption: &str, sections: &[(&str, &str)]) {
    let Some(dir) = evidence_dir() else {
        return;
    };
    let mut body = format!("{caption}\n");
    for (heading, content) in sections {
        body.push_str(&format!("\n--- {heading} ---\n\n"));
        for line in content.lines() {
            body.push_str(&format!("    {line}\n"));
        }
    }
    fs::write(dir.join(format!("{name}.txt")), body).expect("write transcript");
}

/// The headline failure: a document drifted in both directions at once, and a
/// message that names both directions, the offending paths, and where to fix it.
#[test]
fn the_drift_message_names_both_directions_and_the_fix_location() {
    let document = drifted_document();
    let block = verified_block(&document).expect("the synthetic document is well formed");
    let listed = parse_tree(block).expect("the synthetic tree is well formed");

    let message = compare_listings(&listed, &synthetic_source_tree())
        .expect("the synthetic document has drifted in both directions");

    // Direction one: a module was added to the tree and not to the document.
    assert!(
        message.contains("on disk but missing from ARCHITECTURE.md:\n  src/app/reducer/tunnels.rs"),
        "message was: {message}"
    );
    // Direction two: a module was deleted from the tree and left in the document.
    assert!(
        message.contains("listed in ARCHITECTURE.md but not on disk:\n  src/ui/legacy.rs"),
        "message was: {message}"
    );
    // Neither direction may report the other's path, or the two lists would be
    // telling the author to make opposite edits to the same file.
    let missing_half = message
        .split("listed in ARCHITECTURE.md but not on disk:")
        .next()
        .expect("the message has a first half");
    assert!(
        !missing_half.contains("src/ui/legacy.rs"),
        "message was: {message}"
    );

    // The fix location: which document, which section, and the marker names
    // that bound the block inside it.
    assert!(message.contains(ARCHITECTURE_DOC), "message was: {message}");
    assert!(message.contains(SECTION_HEADING), "message was: {message}");
    assert!(message.contains(BEGIN_MARKER), "message was: {message}");
    assert!(message.contains(END_MARKER), "message was: {message}");

    write_transcript(
        "31-architecture-layout-drift-message",
        "31. What an author sees when ARCHITECTURE.md's verified module tree drifts from the \
source tree. The document below is SYNTHETIC and held in memory — the real ARCHITECTURE.md is \
never modified, because this message can only be produced by a broken repository. It has drifted \
in both directions at once: `src/app/reducer/tunnels.rs` was added under `src/` without being \
listed, and `src/ui/legacy.rs` was deleted without being unlisted. The failure separates the two \
directions (a merged list would tell the author to add and remove the same line) and ends with \
the document, the section heading and the marker names, so the fix needs no search.",
        &[
            ("the synthetic ARCHITECTURE.md", &drifted_document()),
            (
                "the synthetic source tree on disk",
                &synthetic_source_tree().join("\n"),
            ),
            ("the failure the author reads", &message),
        ],
    );
}

/// The control: the same synthetic document, un-drifted, is silent. Without
/// this the capture above proves only that the checker can produce a message,
/// not that it produces one because the two listings actually disagree.
#[test]
fn a_document_that_matches_the_tree_produces_no_message() {
    let document = drifted_document().replace("    legacy.rs\n", "").replace(
        "    mod.rs\n  main.rs\n",
        "    mod.rs\n      tunnels.rs\n  main.rs\n",
    );
    let block = verified_block(&document).expect("the repaired document is well formed");
    let listed = parse_tree(block).expect("the repaired tree is well formed");

    assert_eq!(listed, synthetic_source_tree());
    assert_eq!(compare_listings(&listed, &synthetic_source_tree()), None);

    write_transcript(
        "32-architecture-layout-no-drift-is-silent",
        "32. The control for #31: the same synthetic document with the two edits applied — \
`reducer/tunnels.rs` listed, `ui/legacy.rs` removed — parses to exactly the synthetic tree and \
compares clean. The message in #31 is produced by the disagreement, not by the check running.",
        &[
            ("the repaired synthetic ARCHITECTURE.md", &document),
            ("parsed listing", &listed.join("\n")),
            ("comparison result", "no message: the two listings agree"),
        ],
    );
}
