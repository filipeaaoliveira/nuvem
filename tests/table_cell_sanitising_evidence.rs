//! Evidence that resource data cannot carry terminal control through a table
//! cell.
//!
//! Cell text is `gcloud` JSON, and until this pin nothing between the parse
//! and the terminal touched it. What stood in for a policy was ratatui
//! dropping graphemes it considers unprintable as it writes its buffer: an
//! undocumented detail of a dependency, silent about the reordering
//! characters, and worth nothing to any sink that is not a ratatui buffer.
//!
//! `Cell`'s constructors are the choke point, so these assert on the model
//! rather than on rendered output — a frame dump would be asserting ratatui's
//! behaviour again, which is the thing being replaced.

use nuvem::app::projection::table::{Cell, CellKind, StatusSemantic};

/// An escape sequence in a resource name is removed whole, leaving no readable
/// `[31m` residue — the same policy every other rendered surface uses.
#[test]
fn an_escape_sequence_in_a_name_does_not_reach_the_cell() {
    let cell = Cell::plain("web\u{1b}[31m-1");

    assert!(!cell.text.contains('\u{1b}'), "{:?}", cell.text);
    assert!(!cell.text.contains("[31m"), "{:?}", cell.text);
}

/// The reordering characters are what ratatui's grapheme filter never covered:
/// they are printable as far as it is concerned, and they change what the rest
/// of the row says.
#[test]
fn a_right_to_left_override_in_a_name_does_not_reach_the_cell() {
    let cell = Cell::plain("web\u{202E}gnp.txt");

    assert!(!cell.text.contains('\u{202E}'), "{:?}", cell.text);
    assert!(
        cell.text.contains('\u{FFFD}'),
        "a tampered name must look tampered with: {:?}",
        cell.text
    );
}

/// A newline would end the buffer line early and push the rest of the table
/// out of alignment.
#[test]
fn a_newline_in_a_name_does_not_reach_the_cell() {
    let cell = Cell::plain("web\n-1");

    assert!(!cell.text.contains('\n'), "{:?}", cell.text);
}

/// Status text is resource data too — it is `gcloud`'s status string, not a
/// closed vocabulary this crate owns — so it goes through the same door.
#[test]
fn a_status_cell_is_sanitised_like_a_plain_one() {
    let cell = Cell::status("RUN\u{1b}[31mNING", StatusSemantic::Healthy);

    assert!(!cell.text.contains('\u{1b}'), "{:?}", cell.text);
    assert_eq!(cell.kind, CellKind::Status(StatusSemantic::Healthy));
}

/// And the optional constructor, which is how most nullable columns are built.
#[test]
fn an_optional_cell_is_sanitised_when_present() {
    let cell = Cell::optional(Some("10.0.0.2\u{202E}"));

    assert!(!cell.text.contains('\u{202E}'), "{:?}", cell.text);
}

/// The ordinary case must cost nothing: an unremarkable GCP name arrives
/// byte-for-byte, so the guard cannot be rewriting real data.
#[test]
fn an_ordinary_name_is_carried_through_unchanged() {
    assert_eq!(Cell::plain("web-server-1").text, "web-server-1");
    assert_eq!(Cell::plain("europe-west1-b").text, "europe-west1-b");
    assert_eq!(Cell::optional(Some("e2-medium")).text, "e2-medium");
}

/// Non-ASCII text is ordinary too — a label or a description can hold it, and
/// the guard must not mangle it.
#[test]
fn ordinary_unicode_survives_a_cell() {
    assert_eq!(Cell::plain("café-München").text, "café-München");
}
