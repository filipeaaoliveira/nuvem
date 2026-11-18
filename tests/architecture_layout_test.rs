//! Guards the verified module tree in `ARCHITECTURE.md` against the source tree.
//!
//! The parse, compare and format steps live in `support/architecture_layout.rs`
//! as pure functions over a caller-supplied document and a caller-supplied file
//! set; the repository-level tests at the bottom are thin callers that read the
//! real document and walk the real tree. Every path resolves from
//! `CARGO_MANIFEST_DIR`, never from the working directory, so the result cannot
//! differ between a laptop and CI.

use std::fs;

use crate::support::architecture_layout as layout;

use layout::{
    ARCHITECTURE_DOC, BEGIN_MARKER, END_MARKER, SECTION_HEADING, check_roots, compare_listings,
    listed_paths, manifest_dir, parse_tree, source_tree_paths, verified_block,
};

#[test]
fn extracts_the_fence_from_a_document_with_crlf_line_endings() {
    let markdown = "<!-- architecture-layout:verified:begin -->\r\n\r\n```text\r\nsrc/\r\n  main.rs\r\n```\r\n\r\n<!-- architecture-layout:verified:end -->\r\n";

    let block = verified_block(markdown).expect("markers and fence are present");

    assert_eq!(block, "src/\r\n  main.rs\r\n");
    assert_eq!(
        parse_tree(block).expect("carriage returns are tolerated"),
        vec!["src/main.rs"]
    );
}

#[test]
fn skips_a_comment_line_at_the_top_of_the_block() {
    let block = "# VERIFIED - checked by tests/architecture_layout_test.rs\nsrc/\n  main.rs\n";

    let paths = parse_tree(block).expect("comment lines are skipped");

    assert_eq!(paths, vec!["src/main.rs"]);
}

#[test]
fn skips_a_comment_line_between_entries() {
    let block = "src/\n  main.rs\n# a note\n  lib.rs\n";

    let paths = parse_tree(block).expect("comment lines are skipped");

    assert_eq!(paths, vec!["src/main.rs", "src/lib.rs"]);
}

#[test]
fn an_indented_comment_does_not_disturb_the_indent_stack() {
    let block = "src/\n  app/\n      # deeply indented note\n    mod.rs\n  main.rs\n";

    let paths = parse_tree(block).expect("comment lines are skipped before indent checking");

    assert_eq!(paths, vec!["src/app/mod.rs", "src/main.rs"]);
}

#[test]
fn the_indent_jump_message_counts_levels_from_the_line_above() {
    let block = "src/\n  app/\n      mod.rs\n";

    let message = parse_tree(block).expect_err("a two-level indent jump must be rejected");

    assert!(
        message.contains("2 levels deeper"),
        "message was: {message}"
    );
}

/// The nested-under-a-file message reads off the line above the offender. The
/// block's very first entry has no line above it, so blaming one there would
/// print an empty name and send the author looking for a line that is not
/// there.
#[test]
fn rejects_an_indented_first_entry_without_blaming_a_line_above_it() {
    let block = "  src/\n    main.rs\n";

    let message = parse_tree(block).expect_err("the first entry cannot be indented");

    assert!(message.contains("src/"), "message was: {message}");
    assert!(
        message.contains("no line above it"),
        "message was: {message}"
    );
    assert!(
        !message.contains("under ``"),
        "the message must not blame an empty line name: {message}"
    );
}

#[test]
fn rejects_an_entry_nested_under_a_file() {
    let block = "src/\n  a.rs\n    b.rs\n";

    let message = parse_tree(block).expect_err("a file cannot contain entries");

    assert!(message.contains("b.rs"), "message was: {message}");
    assert!(message.contains("a.rs"), "message was: {message}");
    assert!(
        message.contains("not a directory"),
        "message was: {message}"
    );
}

#[test]
fn a_path_listed_twice_is_a_named_failure() {
    let listed = vec!["src/main.rs".to_string(), "src/main.rs".to_string()];
    let on_disk = vec!["src/main.rs".to_string()];

    let message = compare_listings(&listed, &on_disk).expect("a duplicate must be reported");

    assert!(
        message.contains("listed more than once in ARCHITECTURE.md:\n  src/main.rs"),
        "message was: {message}"
    );
    assert!(message.contains(BEGIN_MARKER), "message was: {message}");
}

#[test]
fn check_roots_rejects_a_parent_directory_segment() {
    let listed = vec!["src/../../etc/passwd".to_string()];

    let message = check_roots(&listed).expect("`..` escapes the enforced roots");

    assert!(
        message.contains("src/../../etc/passwd"),
        "message was: {message}"
    );
}

#[test]
fn the_real_verified_block_is_labelled_inside_the_fence() {
    let document = manifest_dir().join(ARCHITECTURE_DOC);
    let markdown = fs::read_to_string(&document)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", document.display()));
    let block = verified_block(&markdown).expect("the verified block is present");

    let first = block
        .lines()
        .find(|line| !line.trim().is_empty())
        .expect("the verified block is not empty");

    assert!(
        first.trim().starts_with("# VERIFIED"),
        "the verified fence must open with a `# VERIFIED` label so its standing is \
         readable from inside the block; first line was: {first}"
    );
    assert!(
        first.contains("tests/architecture_layout_test.rs"),
        "the label must name the checker; first line was: {first}"
    );
}

#[test]
fn parses_a_tree_nested_several_levels_deep() {
    let block = "src/\n  app/\n    reducer/\n      mod.rs\n      logs.rs\n";

    let paths = parse_tree(block).expect("snippet is well formed");

    assert_eq!(
        paths,
        vec!["src/app/reducer/mod.rs", "src/app/reducer/logs.rs"]
    );
}

#[test]
fn parses_a_sibling_that_dedents_back_after_a_nested_block() {
    let block = "src/\n  app/\n    reducer/\n      mod.rs\n    projection.rs\n  main.rs\n";

    let paths = parse_tree(block).expect("snippet is well formed");

    assert_eq!(
        paths,
        vec![
            "src/app/reducer/mod.rs",
            "src/app/projection.rs",
            "src/main.rs"
        ]
    );
}

#[test]
fn parses_both_enforced_roots_in_one_snippet() {
    let block = "crates/nuvem-domain/src/\n  lib.rs\n  gcp/\n    mod.rs\nsrc/\n  main.rs\n";

    let paths = parse_tree(block).expect("snippet is well formed");

    assert_eq!(
        paths,
        vec![
            "crates/nuvem-domain/src/lib.rs",
            "crates/nuvem-domain/src/gcp/mod.rs",
            "src/main.rs"
        ]
    );
}

#[test]
fn rejects_indentation_that_is_not_a_multiple_of_two() {
    let block = "src/\n   main.rs\n";

    let message = parse_tree(block).expect_err("odd indentation must be rejected");

    assert!(message.contains("main.rs"), "message was: {message}");
    assert!(message.contains("indent"), "message was: {message}");
}

#[test]
fn rejects_an_indent_jump_of_two_levels() {
    let block = "src/\n  app/\n      mod.rs\n";

    let message = parse_tree(block).expect_err("a two-level indent jump must be rejected");

    assert!(message.contains("mod.rs"), "message was: {message}");
    assert!(message.contains("one level"), "message was: {message}");
}

#[test]
fn skips_blank_lines_between_entries() {
    let block = "src/\n\n  main.rs\n\n";

    let paths = parse_tree(block).expect("blank lines are skipped");

    assert_eq!(paths, vec!["src/main.rs"]);
}

#[test]
fn extracts_the_text_fence_between_the_markers() {
    let markdown = "# Doc\n\nprose\n\n<!-- architecture-layout:verified:begin -->\n\n```text\nsrc/\n  main.rs\n```\n\n<!-- architecture-layout:verified:end -->\n\nmore prose\n\n```text\nillustrative/\n```\n";

    let block = verified_block(markdown).expect("markers and fence are present");

    assert_eq!(block, "src/\n  main.rs\n");
}

#[test]
fn rejects_a_document_missing_the_begin_marker() {
    let markdown =
        "# Doc\n\n```text\nsrc/\n  main.rs\n```\n\n<!-- architecture-layout:verified:end -->\n";

    let message = verified_block(markdown).expect_err("a missing marker must be rejected");

    assert!(
        message.contains("architecture-layout:verified:begin"),
        "message was: {message}"
    );
    assert!(
        message.contains("ARCHITECTURE.md"),
        "message was: {message}"
    );
}

#[test]
fn rejects_two_text_fences_between_the_markers() {
    let markdown = "<!-- architecture-layout:verified:begin -->\n```text\nsrc/\n```\n```text\nsrc/\n```\n<!-- architecture-layout:verified:end -->\n";

    let message = verified_block(markdown).expect_err("two fences must be rejected");

    assert!(message.contains("exactly one"), "message was: {message}");
}

#[test]
fn rejects_markers_without_a_text_fence() {
    let markdown = "<!-- architecture-layout:verified:begin -->\nsrc/\n<!-- architecture-layout:verified:end -->\n";

    let message = verified_block(markdown).expect_err("a missing fence must be rejected");

    assert!(message.contains("```text"), "message was: {message}");
}

#[test]
fn identical_listings_compare_clean() {
    let listed = vec!["src/main.rs".to_string(), "src/lib.rs".to_string()];
    let on_disk = vec!["src/lib.rs".to_string(), "src/main.rs".to_string()];

    assert_eq!(compare_listings(&listed, &on_disk), None);
}

#[test]
fn comparison_reports_both_directions_and_the_fix_location() {
    let listed = vec!["src/ghost.rs".to_string()];
    let on_disk = vec!["src/main.rs".to_string()];

    let message = compare_listings(&listed, &on_disk).expect("the listings differ");

    assert!(
        message.contains("on disk but missing from ARCHITECTURE.md:\n  src/main.rs"),
        "message was: {message}"
    );
    assert!(
        message.contains("listed in ARCHITECTURE.md but not on disk:\n  src/ghost.rs"),
        "message was: {message}"
    );
    assert!(message.contains(ARCHITECTURE_DOC), "message was: {message}");
    assert!(message.contains(SECTION_HEADING), "message was: {message}");
    assert!(message.contains(BEGIN_MARKER), "message was: {message}");
    assert!(message.contains(END_MARKER), "message was: {message}");
}

#[test]
fn comparison_prints_none_for_an_empty_direction() {
    let listed: Vec<String> = Vec::new();
    let on_disk = vec!["src/main.rs".to_string()];

    let message = compare_listings(&listed, &on_disk).expect("the listings differ");

    assert!(
        message.contains("listed in ARCHITECTURE.md but not on disk:\n  (none)"),
        "message was: {message}"
    );
}

#[test]
fn a_path_outside_the_enforced_roots_is_a_failure_naming_the_illustrative_block() {
    let listed = vec!["src/main.rs".to_string(), "tests/cli_test.rs".to_string()];

    let message = check_roots(&listed).expect("tests/ is outside the enforced roots");

    assert!(
        message.contains("tests/cli_test.rs"),
        "message was: {message}"
    );
    assert!(message.contains("illustrative"), "message was: {message}");
    assert!(message.contains(ARCHITECTURE_DOC), "message was: {message}");
}

#[test]
fn paths_under_the_enforced_roots_pass_the_root_check() {
    let listed = vec![
        "src/main.rs".to_string(),
        "crates/nuvem-domain/src/lib.rs".to_string(),
    ];

    assert_eq!(check_roots(&listed), None);
}

#[test]
fn architecture_verified_tree_lists_only_enforced_roots() {
    let listed = listed_paths();

    if let Some(message) = check_roots(&listed) {
        panic!("{message}");
    }
}

#[test]
fn architecture_verified_tree_matches_source_tree() {
    let listed = listed_paths();
    let on_disk = source_tree_paths();

    if let Some(message) = compare_listings(&listed, &on_disk) {
        panic!("{message}");
    }
}
