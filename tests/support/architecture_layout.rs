//! Shared support for the verified module tree in `ARCHITECTURE.md`.
//!
//! The parse, compare and format steps are pure functions over a caller-supplied
//! document and a caller-supplied file set. `tests/architecture_layout_test.rs`
//! calls them against the real document and the real tree;
//! `tests/architecture_layout_evidence.rs` calls them against a synthetic pair to
//! capture what an author sees when the two disagree. Both pull this file in
//! with `#[path = "support/architecture_layout.rs"]`, so it lives in a
//! subdirectory: a plain `tests/*.rs` file would become a test target of its own.
//!
//! Every path resolves from `CARGO_MANIFEST_DIR`, never from the working
//! directory, so the result cannot differ between a laptop and CI.
//!
//! Each binary uses a different subset of what is here, so the unused half
//! would otherwise be reported as dead code in whichever one does not call it.
#![allow(dead_code)]

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Document that owns the verified tree, relative to the crate manifest.
pub(crate) const ARCHITECTURE_DOC: &str = "ARCHITECTURE.md";
/// Heading of the section that owns the verified tree.
pub(crate) const SECTION_HEADING: &str = "## Repository Layout";
pub(crate) const BEGIN_MARKER: &str = "<!-- architecture-layout:verified:begin -->";
pub(crate) const END_MARKER: &str = "<!-- architecture-layout:verified:end -->";
pub(crate) const FENCE_OPEN: &str = "```text";
pub(crate) const FENCE_CLOSE: &str = "```";
/// The only roots the verified block may describe.
pub(crate) const ENFORCED_ROOTS: [&str; 2] = ["crates/nuvem-domain/src", "src"];

/// Where an author must go to fix any failure this file reports.
pub(crate) fn fix_location() -> String {
    format!(
        "Fix it in {ARCHITECTURE_DOC}, section \"{SECTION_HEADING}\", in the fenced\n\
         block between the {BEGIN_MARKER}\n\
         and {END_MARKER} markers.\n\
         Every `.rs` file under `src/` and `crates/nuvem-domain/src/` appears there\n\
         exactly once, and nothing else does."
    )
}

/// Extracts the ```text fence sitting between the two verified markers.
pub(crate) fn verified_block(markdown: &str) -> Result<&str, String> {
    let begin = markdown.find(BEGIN_MARKER).ok_or_else(|| {
        format!(
            "{ARCHITECTURE_DOC} has no {BEGIN_MARKER} marker.\n\n{}",
            fix_location()
        )
    })?;
    let after_begin = begin + BEGIN_MARKER.len();
    let end = markdown[after_begin..]
        .find(END_MARKER)
        .map(|offset| after_begin + offset)
        .ok_or_else(|| {
            format!(
                "{ARCHITECTURE_DOC} has no {END_MARKER} marker after the begin marker.\n\n{}",
                fix_location()
            )
        })?;
    let region = &markdown[after_begin..end];

    // Byte spans, not `lines()`: `lines()` strips a `\r`, and re-deriving offsets
    // from it misaligns the slice by one byte per line on a CRLF document.
    let spans = line_spans(region);
    let opens: Vec<usize> = spans
        .iter()
        .enumerate()
        .filter(|&(_, &(start, stop))| region[start..stop].trim_end() == FENCE_OPEN)
        .map(|(index, _)| index)
        .collect();
    if opens.len() != 1 {
        return Err(format!(
            "expected exactly one {FENCE_OPEN} fence between the verified markers in \
             {ARCHITECTURE_DOC}, found {}.\n\n{}",
            opens.len(),
            fix_location()
        ));
    }

    let open = opens[0];
    let close = spans
        .iter()
        .enumerate()
        .skip(open + 1)
        .find(|&(_, &(start, stop))| region[start..stop].trim() == FENCE_CLOSE)
        .map(|(index, _)| index);

    match close {
        Some(close) => Ok(&region[spans[open + 1].0..spans[close].0]),
        None => Err(format!(
            "the {FENCE_OPEN} fence between the verified markers in {ARCHITECTURE_DOC} is \
             not closed.\n\n{}",
            fix_location()
        )),
    }
}

/// Byte spans of each line of `text`, excluding the `\n` and including any `\r`.
fn line_spans(text: &str) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut start = 0usize;
    for (index, byte) in text.bytes().enumerate() {
        if byte == b'\n' {
            spans.push((start, index));
            start = index + 1;
        }
    }
    if start < text.len() {
        spans.push((start, text.len()));
    }
    spans
}

/// Turns a two-space-indented tree into `/`-joined relative paths (files only).
pub(crate) fn parse_tree(block: &str) -> Result<Vec<String>, String> {
    let mut stack: Vec<&str> = Vec::new();
    let mut paths = Vec::new();
    let mut previous_level = 0usize;
    let mut previous_entry = "";

    for line in block.lines() {
        let entry = line.trim();
        // A `#` line labels the block's standing; it is not an entry, and it does
        // not participate in the indent stack.
        if entry.is_empty() || entry.starts_with('#') {
            continue;
        }
        let indent = line.len() - line.trim_start_matches(' ').len();
        if line.contains('\t') {
            return Err(format!(
                "tab in the indentation of `{}`; the verified tree indents with two \
                 spaces per level.\n\n{}",
                line.trim(),
                fix_location()
            ));
        }
        if indent % 2 != 0 {
            return Err(format!(
                "`{}` has an indent of {indent} spaces; the verified tree indents with two \
                 spaces per level.\n\n{}",
                line.trim(),
                fix_location()
            ));
        }
        let level = indent / 2;
        if level > previous_level + 1 {
            return Err(format!(
                "`{entry}` indents {} levels deeper than the line above it; the verified tree \
                 may only descend one level at a time.\n\n{}",
                level - previous_level,
                fix_location()
            ));
        }
        if level > stack.len() {
            // `previous_entry` is empty only on the block's first entry, where
            // there is no line above to have opened a directory.
            return Err(if previous_entry.is_empty() {
                format!(
                    "`{entry}` is indented, but it is the first entry in the verified tree, so \
                     there is no line above it to nest under; the tree's top-level entries start \
                     at column zero.\n\n{}",
                    fix_location()
                )
            } else {
                format!(
                    "`{entry}` is indented under `{previous_entry}`, which is a file, not a \
                     directory.\n\n{}",
                    fix_location()
                )
            });
        }
        stack.truncate(level);
        previous_level = level;
        previous_entry = entry;

        match entry.strip_suffix('/') {
            Some(directory) => stack.push(directory),
            None => {
                let mut path = String::new();
                for segment in &stack {
                    path.push_str(segment);
                    path.push('/');
                }
                path.push_str(entry);
                paths.push(path);
            }
        }
    }

    Ok(paths)
}

/// Rejects any listed path that is not under an enforced root.
pub(crate) fn check_roots(listed: &[String]) -> Option<String> {
    let strays: Vec<&String> = listed
        .iter()
        .filter(|path| {
            // A `..` segment escapes the root it appears to sit under, so a prefix
            // match alone would not confine the path.
            path.split('/').any(|segment| segment == "..")
                || !ENFORCED_ROOTS
                    .iter()
                    .any(|root| path.starts_with(&format!("{root}/")))
        })
        .collect();
    if strays.is_empty() {
        return None;
    }

    let mut message = String::from(
        "The verified module tree lists paths outside the roots it can check.\n\n\
         outside `src/` and `crates/nuvem-domain/src/`:\n",
    );
    for stray in strays {
        message.push_str("  ");
        message.push_str(stray);
        message.push('\n');
    }
    message.push_str(
        "\nMove these to the illustrative supporting-files block, which is not \
         machine-checked.\n\n",
    );
    message.push_str(&fix_location());
    Some(message)
}

/// Rejects a path the verified block lists more than once. A set difference
/// swallows duplicates, and the block promises each file appears exactly once.
fn check_duplicates(listed: &[String]) -> Option<String> {
    let mut seen = BTreeSet::new();
    let mut duplicates = BTreeSet::new();
    for path in listed {
        if !seen.insert(path.as_str()) {
            duplicates.insert(path.as_str());
        }
    }
    if duplicates.is_empty() {
        return None;
    }

    let mut message = String::from(
        "The verified module tree lists a path more than once.\n\n\
         listed more than once in ARCHITECTURE.md:\n",
    );
    for path in duplicates {
        message.push_str("  ");
        message.push_str(path);
        message.push('\n');
    }
    message.push('\n');
    message.push_str(&fix_location());
    Some(message)
}

/// Compares the two listings in both directions, returning a failure message.
pub(crate) fn compare_listings(listed: &[String], on_disk: &[String]) -> Option<String> {
    if let Some(message) = check_duplicates(listed) {
        return Some(message);
    }
    let listed: BTreeSet<&str> = listed.iter().map(String::as_str).collect();
    let on_disk: BTreeSet<&str> = on_disk.iter().map(String::as_str).collect();

    let missing: Vec<&&str> = on_disk.difference(&listed).collect();
    let extra: Vec<&&str> = listed.difference(&on_disk).collect();
    if missing.is_empty() && extra.is_empty() {
        return None;
    }

    let mut message = String::from(
        "The verified module tree in ARCHITECTURE.md has drifted from the source tree.\n\n",
    );
    push_list(
        &mut message,
        "on disk but missing from ARCHITECTURE.md:",
        &missing,
    );
    push_list(
        &mut message,
        "listed in ARCHITECTURE.md but not on disk:",
        &extra,
    );
    message.push_str(&fix_location());
    Some(message)
}

fn push_list(message: &mut String, heading: &str, paths: &[&&str]) {
    message.push_str(heading);
    message.push('\n');
    if paths.is_empty() {
        message.push_str("  (none)\n");
    } else {
        for path in paths {
            message.push_str("  ");
            message.push_str(path);
            message.push('\n');
        }
    }
    message.push('\n');
}

/// Collects `*.rs` files under `directory`, as `/`-joined paths below `prefix`.
fn collect_rs_files(directory: &Path, prefix: &str, out: &mut Vec<String>) {
    let entries = fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", directory.display()));
    for entry in entries {
        let entry = entry.unwrap_or_else(|error| {
            panic!("cannot read an entry of {}: {error}", directory.display())
        });
        let name = entry.file_name().to_string_lossy().into_owned();
        let child = format!("{prefix}/{name}");
        let file_type = entry
            .file_type()
            .unwrap_or_else(|error| panic!("cannot stat {child}: {error}"));
        if file_type.is_dir() {
            collect_rs_files(&entry.path(), &child, out);
        } else if name.ends_with(".rs") {
            out.push(child);
        }
    }
}

pub(crate) fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Every `.rs` file under the enforced roots of the real repository.
pub(crate) fn source_tree_paths() -> Vec<String> {
    let mut paths = Vec::new();
    for root in ENFORCED_ROOTS {
        let directory = manifest_dir().join(root);
        collect_rs_files(&directory, root, &mut paths);
    }
    paths.sort();
    paths
}

/// Every file path listed in the real document's verified block.
pub(crate) fn listed_paths() -> Vec<String> {
    let document = manifest_dir().join(ARCHITECTURE_DOC);
    let markdown = fs::read_to_string(&document)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", document.display()));
    let block = match verified_block(&markdown) {
        Ok(block) => block,
        Err(message) => panic!("{message}"),
    };
    match parse_tree(block) {
        Ok(paths) => paths,
        Err(message) => panic!("{message}"),
    }
}
