//! What nuvem's exit status tells a script.
//!
//! nuvem is a TUI, but two of its entry points are things a script runs:
//! `nuvem info` prints an environment report and exits, and `--demo` makes the
//! whole app runnable in CI against fixtures. Both used to report every
//! failure as `1`, which is what `fn main() -> anyhow::Result<()>` produces —
//! so "there is no terminal here", "the log file could not be opened", and
//! "the session ended badly" were indistinguishable to the caller.
//!
//! The codes below are a promise, so they are few and they do not move. A
//! failure that does not match a named category stays [`RUNTIME`], which is
//! the `1` everything reported before: adding a category is a compatible
//! change, renumbering one is not.
//!
//! [`USAGE`] is not produced by this module. `clap` exits with it directly on
//! a bad flag, before `main` runs at all; it is written down here because the
//! table a script reads has to be in one place, and a reader who finds three
//! codes documented and a fourth in the wild would reasonably conclude the
//! table was wrong.

use std::fmt;

/// The session ran and ended in failure, or failed for a reason with no
/// category of its own. The status every failure carried before these codes
/// existed.
pub const RUNTIME: u8 = 1;

/// A bad flag or an unknown subcommand. Produced by `clap`, not by this
/// module; see the module docs.
pub const USAGE: u8 = 2;

/// stdin or stdout is not a terminal, so there is nothing to draw on.
///
/// Its own code because it is the one failure a caller is most likely to have
/// caused on purpose: a pipeline, a cron job, or a CI step that ran nuvem
/// without a TTY. Told apart from a real failure, it is a condition a script
/// can handle; folded into `1`, it looks like the app broke.
pub const NO_TERMINAL: u8 = 3;

/// nuvem could not start: the log file could not be created or opened, or
/// another startup resource was unavailable.
///
/// Distinct from [`RUNTIME`] because nothing has run yet — no gcloud command
/// was issued and no cloud state was read, so a caller retrying is retrying a
/// local problem, not a remote one.
pub const STARTUP: u8 = 4;

/// A failure that knows which exit status it should produce.
///
/// It carries the operator-facing message itself rather than sitting under one
/// as a cause. An earlier version wrapped the real message with
/// `.context(…)`, which put the category *in* the chain — and anyhow renders a
/// chain, so the terminal refusal reached the operator as
///
/// ```text
/// Error: nuvem requires an interactive terminal
///
/// Caused by:
///     exit status 3
/// ```
///
/// reading as though the status were the reason for the failure rather than
/// its consequence. One error with one message and a code attached to it says
/// the same thing without the second paragraph.
#[derive(Debug)]
pub struct Categorised {
    code: u8,
    message: String,
    source: Option<Box<dyn std::error::Error + Send + Sync + 'static>>,
}

impl Categorised {
    /// A failure that is fully described by its own message.
    pub fn new(code: u8, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            source: None,
        }
    }

    /// A failure with an underlying cause worth showing.
    ///
    /// The distinction from [`Categorised::new`] is about what the operator
    /// gains from a second paragraph. "exit status 3" underneath "nuvem
    /// requires an interactive terminal" is noise — the category restated.
    /// "Not a directory (os error 20)" underneath "could not open the log file
    /// /x/y" is the part that says what to change, so it is kept as a real
    /// source and rendered as a real cause. It also stays reachable to
    /// `downcast_ref`, which is how callers tell one startup failure from
    /// another.
    pub fn with_source(
        code: u8,
        message: impl Into<String>,
        source: impl Into<Box<dyn std::error::Error + Send + Sync + 'static>>,
    ) -> Self {
        Self {
            code,
            message: message.into(),
            source: Some(source.into()),
        }
    }

    pub fn code(&self) -> u8 {
        self.code
    }
}

impl fmt::Display for Categorised {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Categorised {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_ref()
            .map(|source| &**source as &(dyn std::error::Error + 'static))
    }
}

/// The status to exit with for `error`.
///
/// Reads the category off the error chain, so a failure keeps its code
/// through any number of `.context(…)` calls between where it happened and
/// where it is reported. Anything uncategorised is [`RUNTIME`].
pub fn code_for(error: &anyhow::Error) -> u8 {
    error
        .chain()
        .find_map(|cause| cause.downcast_ref::<Categorised>())
        .map(Categorised::code)
        .unwrap_or(RUNTIME)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_uncategorised_failure_keeps_the_status_it_always_had() {
        assert_eq!(code_for(&anyhow::anyhow!("something broke")), RUNTIME);
        assert_eq!(RUNTIME, 1, "the pre-existing status must not move");
    }

    #[test]
    fn a_categorised_failure_reports_its_own_status() {
        let error = anyhow::Error::new(Categorised::new(NO_TERMINAL, "no terminal"));

        assert_eq!(code_for(&error), NO_TERMINAL);
    }

    /// The message an operator reads is the one the failure supplied, with
    /// nothing of this module's own added to it — the whole reason the code
    /// travels inside the error rather than as a link above it.
    #[test]
    fn the_rendered_message_is_the_failures_own() {
        let categorised = Categorised::new(STARTUP, "could not open the log file");

        assert_eq!(categorised.to_string(), "could not open the log file");
    }

    /// A cause worth showing is kept, and stays reachable to `downcast_ref`
    /// so callers can still tell one startup failure from another.
    #[test]
    fn a_source_is_carried_and_stays_downcastable() {
        let io = std::io::Error::new(std::io::ErrorKind::NotADirectory, "not a directory");
        let categorised = Categorised::with_source(STARTUP, "could not open the log file", io);

        assert_eq!(code_for(&anyhow::Error::new(categorised)), STARTUP);

        let categorised = Categorised::with_source(
            STARTUP,
            "could not open the log file",
            std::io::Error::new(std::io::ErrorKind::NotADirectory, "not a directory"),
        );
        let source = std::error::Error::source(&categorised)
            .expect("the underlying failure is kept as a real source");
        let found = source
            .downcast_ref::<std::io::Error>()
            .expect("and stays reachable by type");
        assert_eq!(found.kind(), std::io::ErrorKind::NotADirectory);
    }

    /// And it stays the only paragraph: rendered through anyhow, a
    /// categorised failure has no `Caused by` section to explain away.
    #[test]
    fn a_categorised_failure_renders_as_one_paragraph() {
        let error = anyhow::Error::new(Categorised::new(NO_TERMINAL, "no terminal here"));

        let rendered = format!("{error:?}");

        assert!(
            !rendered.contains("Caused by"),
            "the category must not read as the cause: {rendered}"
        );
        assert!(rendered.contains("no terminal here"), "{rendered}");
    }

    /// The reason the category rides the chain rather than the top-level
    /// error: every caller between the failure and `main` is free to add
    /// context, and none of them should have to know about exit codes.
    #[test]
    fn context_added_on_the_way_up_does_not_lose_the_category() {
        let error = anyhow::Error::new(Categorised::new(STARTUP, "could not open the log file"))
            .context("starting up");

        assert_eq!(code_for(&error), STARTUP);
    }

    /// Each code names one condition, and a script switching on them needs
    /// them to stay distinct.
    #[test]
    fn the_categories_are_distinct() {
        let codes = [RUNTIME, USAGE, NO_TERMINAL, STARTUP];
        let mut sorted = codes;
        sorted.sort_unstable();
        let mut deduped = sorted.to_vec();
        deduped.dedup();

        assert_eq!(deduped.len(), codes.len(), "codes collide: {codes:?}");
    }

    /// Pinned against literals rather than against each other: the point of
    /// the table is that these exact numbers are what a script reads, so a
    /// test that compared constants to constants would move with them.
    #[test]
    fn the_published_codes_are_the_documented_ones() {
        assert_eq!((RUNTIME, USAGE, NO_TERMINAL, STARTUP), (1, 2, 3, 4));
    }
}
