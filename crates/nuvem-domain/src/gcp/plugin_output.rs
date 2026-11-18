//! What a plugin printed, kept readable.
//!
//! [`crate::gcp::diagnostics::sanitize_detail`] is the right shape for a
//! status line and the wrong one for output: it joins on whitespace, so every
//! line break is gone, and it bounds at [`crate::gcp::diagnostics::DETAIL_MAX_LEN`]
//! bytes. A `describe` rendered through it is one run-on line cut mid-word.
//!
//! This module keeps the same redaction and drops the flattening.
//!
//! **[`PluginOutput::capture`] is the only place plugin output is sanitised**,
//! and deliberately so. A plugin's stdout is the most attacker-influenced text
//! nuvem displays — its `args` are the operator's, but what comes back is
//! whatever the command printed — and it reaches two sinks with different
//! policies, the viewer and the clipboard. Sanitising at each sink would mean
//! two chances to forget; sanitising once, here, means the sinks can render
//! [`OutputLine::text`] as-is because there is nothing left in it to remove.
//! Anything that grows a third sink inherits that, and anything that adds a
//! sanitiser at a sink is adding a no-op. Escapes are gone, control characters
//! are gone or spaced, and credential-shaped tokens are `[redacted]` before a
//! line is ever stored.

use crate::gcp::diagnostics::redact_token;
use crate::gcp::text::strip_terminal_escapes;

/// How much of one run's output is retained, in bytes of display text.
///
/// `args` is the operator's, so the volume a plugin prints is theirs too and
/// nuvem cannot hold all of it: ten records at this bound is the ceiling.
/// Past it the HEAD is kept, because gcloud puts the identifying fields
/// first — a truncated `describe` that still names what it described is worth
/// more than its closing braces.
pub const PLUGIN_OUTPUT_MAX_BYTES: usize = 256 * 1024;

/// Bytes of the bound kept back for stderr, so a flood on stdout cannot
/// silence a warning.
///
/// A floor, not a second budget: stderr reserves only what it can use, and
/// whatever stdout leaves is added to it afterwards, so the two streams
/// together still fit [`PLUGIN_OUTPUT_MAX_BYTES`]. Without this, a
/// `--format=json` listing over the bound spent the whole budget first and
/// every stderr line fell into the drop branch — the case where a quota or
/// deprecation warning matters most.
const STDERR_RESERVE: usize = 32 * 1024;

/// Which stream a line came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputStream {
    Stdout,
    Stderr,
}

/// One retained line of a plugin's output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputLine {
    pub text: String,
    pub stream: OutputStream,
}

/// A plugin run's captured output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginOutput {
    lines: Vec<OutputLine>,
    dropped_bytes: usize,
}

impl PluginOutput {
    /// Captures both streams of one run, redacted, bounded, head first.
    pub fn capture(stdout: &str, stderr: &str) -> Self {
        let mut output = Self {
            lines: Vec::new(),
            dropped_bytes: 0,
        };
        let reserved_for_stderr = stderr.len().min(STDERR_RESERVE);
        let mut budget = PLUGIN_OUTPUT_MAX_BYTES - reserved_for_stderr;
        output.push_stream(stdout, OutputStream::Stdout, &mut budget);
        budget += reserved_for_stderr;
        output.push_stream(stderr, OutputStream::Stderr, &mut budget);
        output
    }

    /// The captured lines, stdout before stderr, each already redacted.
    pub fn lines(&self) -> &[OutputLine] {
        &self.lines
    }

    /// Bytes of display text the bound refused.
    ///
    /// Counted after redaction, because that is the text this type would have
    /// shown: it describes what the viewer is missing, not what `gcloud`
    /// printed.
    pub fn dropped_bytes(&self) -> usize {
        self.dropped_bytes
    }

    /// Whether the bound refused anything, i.e. whether the viewer is showing
    /// less than the run produced.
    pub fn is_truncated(&self) -> bool {
        self.dropped_bytes > 0
    }

    /// Bytes as something an operator reads, for the truncation line the
    /// viewer draws.
    pub fn dropped_bytes_display(&self) -> String {
        const KIB: usize = 1024;
        const MIB: usize = KIB * 1024;
        match self.dropped_bytes {
            0..KIB => format!("{} B", self.dropped_bytes),
            KIB..MIB => format!("{} KiB", self.dropped_bytes / KIB),
            _ => format!("{} MiB", self.dropped_bytes / MIB),
        }
    }

    fn push_stream(&mut self, text: &str, stream: OutputStream, budget: &mut usize) {
        // Per stream, not per capture: a stdout that ran out of budget must
        // not also cut stderr short of its reserve. Within one stream it is
        // the contiguity rule — nothing after the cut, so the head has no
        // holes in it.
        let mut cut = false;

        for line in text.lines() {
            // Escapes are stripped *before* redaction, and the order is
            // load-bearing for the same reason it is in
            // `sanitize_rendered_paragraph`: redaction removes control
            // characters, so it destroys the `ESC` that introduces a sequence
            // while leaving the rest of it as ordinary text — `ESC[31m`
            // reaching redaction first arrives on screen and on the clipboard
            // as a readable `[31m`. Stripping first means the whole sequence
            // goes, or none of it does.
            let text = redact_line(&strip_terminal_escapes(line));
            // The newline is charged too: it is a byte of the output, and
            // omitting it makes the bound drift by one per line.
            let cost = text.len() + 1;

            // Everything after the cut is dropped, including lines that would
            // still have fitted. A head with a hole in it is worse than a
            // shorter head: nothing on screen would say where the hole was.
            if cost > *budget || cut {
                // The line that straddles the cut is kept up to the budget
                // rather than dropped whole: `gcloud --format=json` prints
                // everything on one line, and dropping it would leave the
                // viewer empty for the commonest machine-readable output
                // there is.
                if !cut && *budget > 0 {
                    let head = char_boundary_prefix(&text, *budget);
                    self.dropped_bytes += cost - head.len();
                    self.lines.push(OutputLine {
                        text: head.to_string(),
                        stream,
                    });
                    *budget = 0;
                    cut = true;
                    continue;
                }
                cut = true;
                self.dropped_bytes += cost;
                continue;
            }

            *budget -= cost;
            self.lines.push(OutputLine { text, stream });
        }
    }
}

/// The longest prefix of `text` that fits `budget` bytes without splitting a
/// char. Slicing on a byte index alone panics on multi-byte content, and a
/// plugin's output is not this repo's to keep ASCII.
fn char_boundary_prefix(text: &str, budget: usize) -> &str {
    if text.len() <= budget {
        return text;
    }
    let mut end = budget;
    // No `end > 0` guard: byte 0 is always a char boundary, so the walk stops
    // there at the latest and cannot underflow. Guarding it anyway would be a
    // condition no input can make false — unreachable by construction, and so
    // unpinnable by any test.
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// Redacts a single line, preserving the whitespace between its tokens.
///
/// `sanitize_detail` collapses runs of whitespace because a status line has
/// one line to spend. Output is read by its shape — YAML nesting, a `list`'s
/// columns — so the runs are kept and only the tokens between them are
/// rewritten.
fn redact_line(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;

    while !rest.is_empty() {
        let token_at = rest
            .find(|c: char| !c.is_whitespace())
            .unwrap_or(rest.len());
        // A whitespace run is kept for its width, not copied verbatim: `\r`,
        // `\x0b` and `\x0c` are all whitespace, and a lone `\r` would rewrite
        // a line the terminal has already drawn. Each becomes one space, so
        // the run still occupies what it occupied. A tab is control too and
        // becomes a space with the rest — ratatui has no tab stops to honour.
        out.extend(
            rest[..token_at]
                .chars()
                .map(|c| if c.is_control() { ' ' } else { c }),
        );
        rest = &rest[token_at..];

        if rest.is_empty() {
            break;
        }

        let token_end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        out.push_str(&redact_token(&rest[..token_end]));
        rest = &rest[token_end..];
    }

    out
}
