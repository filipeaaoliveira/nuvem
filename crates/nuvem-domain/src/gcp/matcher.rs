//! The matching primitives both filter grammars are built from.
//!
//! Two surfaces above this module let the user type a one-line filter: the
//! resource table (`gcp::filter`) and the log viewer (`gcp::text_filter`).
//! They used to carry a private copy each of the same three things — a
//! case-insensitive regex, a subsequence test, and an inverse flag — which is
//! how the two grammars drifted apart in the details while looking identical
//! in the docs. The log surface additionally had a lowercased substring test
//! of its own, the table grammar having no substring mode at all. All four
//! primitives live here once now; the grammars above are only parsers that
//! decide *which* primitive an input selects.
//!
//! This module is surface-neutral on purpose: it knows about strings, not
//! about logs or resources.
//!
//! **The two surface defaults are deliberately different, and consolidating
//! them here does not make them the same.** An unprefixed table filter is a
//! regex; an unprefixed log filter is a literal substring. That asymmetry is
//! a decision, not an oversight: the single most common log search is a
//! bracketed level such as `[ERROR]`, which under regex is a character class
//! that matches every line containing an `E`, `R` or `O` and never matches
//! the brackets the user typed. The table has no such idiom and gains real
//! power from regex-by-default. Anyone tempted to "finish the job" by giving
//! both surfaces one default should read that sentence twice.

use regex::{Regex, RegexBuilder};

/// One compiled way of testing a haystack string.
///
/// Needles are lowercased once at construction, so matching only has to
/// lowercase the haystack. Regexes carry their case-insensitivity in the
/// compiled program instead.
#[derive(Debug, Clone)]
pub enum Matcher {
    /// An empty pattern of any syntax — it accepts every haystack. The
    /// grammars above still remember which syntax produced it, so the UI can
    /// keep reporting the mode the user typed.
    Everything,
    Substring(String),
    Fuzzy(Vec<char>),
    Regex(Regex),
}

impl Matcher {
    /// A literal case-insensitive substring test. An empty needle is
    /// [`Matcher::Everything`].
    pub fn substring(needle: &str) -> Self {
        if needle.is_empty() {
            Self::Everything
        } else {
            Self::Substring(needle.to_lowercase())
        }
    }

    /// A case-insensitive subsequence test. An empty needle is
    /// [`Matcher::Everything`].
    ///
    /// The needle is lowercased as a whole string *before* being split into
    /// chars, never char by char: some chars (`İ`) lowercase to two, and the
    /// haystack has to supply both of them for the match to hold.
    pub fn fuzzy(needle: &str) -> Self {
        if needle.is_empty() {
            Self::Everything
        } else {
            Self::Fuzzy(needle.to_lowercase().chars().collect())
        }
    }

    /// A case-insensitive regex. An empty pattern is [`Matcher::Everything`].
    ///
    /// Case-insensitivity is a builder flag rather than a `(?i)` prefix glued
    /// onto the user's pattern. The two are equivalent to the regex engine,
    /// including the way an inline `(?-i)` in the user's own pattern overrides
    /// them both — but the flag keeps the pattern the user typed intact, so
    /// error spans point at their text and not at four characters we added.
    pub fn regex(pattern: &str) -> Result<Self, regex::Error> {
        if pattern.is_empty() {
            return Ok(Self::Everything);
        }
        Ok(Self::Regex(
            RegexBuilder::new(pattern).case_insensitive(true).build()?,
        ))
    }

    /// True when this matcher accepts `haystack`.
    pub fn matches(&self, haystack: &str) -> bool {
        match self {
            Self::Everything => true,
            Self::Substring(needle) => haystack.to_lowercase().contains(needle),
            Self::Fuzzy(needle) => is_subsequence(needle, &haystack.to_lowercase()),
            Self::Regex(regex) => regex.is_match(haystack),
        }
    }
}

/// A [`Matcher`] plus the inverse flag: the whole of what a parsed filter
/// evaluates.
///
/// This is the *only* implementation of inverse in the codebase. Both
/// grammars route their `!` through it, so `!` cannot come to mean one thing
/// on the log surface and another on the table surface — the two share the
/// code, not just the documentation.
///
/// What the surfaces do *before* building a `Predicate` is still theirs. The
/// table treats a prefix with an empty remainder as "no filter yet" and so
/// never builds an inverted empty predicate at all, while a bare `!` in the
/// log viewer builds one and matches nothing. That difference lives in the
/// parsers, where it is visible, rather than here.
#[derive(Debug, Clone)]
pub struct Predicate {
    matcher: Matcher,
    inverted: bool,
}

impl Predicate {
    pub fn new(matcher: Matcher, inverted: bool) -> Self {
        Self { matcher, inverted }
    }

    /// True when any of `haystacks` matches, XOR-ed with the inverse flag. An
    /// empty haystack list never matches, so it yields the inverse flag
    /// itself.
    pub fn matches_any<'a>(&self, haystacks: impl IntoIterator<Item = &'a str>) -> bool {
        let hit = haystacks
            .into_iter()
            .any(|haystack| self.matcher.matches(haystack));
        hit ^ self.inverted
    }

    /// True when the predicate accepts every haystack and callers may skip
    /// filtering altogether. Inverting an empty pattern matches *nothing*, so
    /// it is never match-all.
    pub fn is_match_all(&self) -> bool {
        !self.inverted && matches!(self.matcher, Matcher::Everything)
    }

    /// Whether the pattern was negated. For a surface that wants to render
    /// the `!` it parsed; [`Predicate::matches_any`] has already applied it.
    pub fn is_inverted(&self) -> bool {
        self.inverted
    }
}

/// The single meaningful line of a `regex` crate error.
///
/// A parse error's `Display` is a four-line block — a `regex parse error:`
/// header, the notated pattern, an ASCII-art caret, then `error: <reason>` —
/// of which only the last line says anything. Both surfaces render this
/// reason inline in a header that is truncated to the terminal width, so a
/// multi-line block pushes the reason off-screen and leaves a caret pointing
/// at nothing. We take the reason and drop the rest.
///
/// The fallback is deliberately the *first non-empty line* rather than the
/// whole rendered text: an error shaped differently from a parse error (a
/// compiled size limit, say) is already one line, and anything else that
/// might arrive must still be safe to drop into a one-line header.
pub fn regex_reason(error: &regex::Error) -> String {
    let rendered = error.to_string();
    // The scan runs in REVERSE and must stay that way. The reason is the last
    // line of the block, and the line above the caret echoes the user's own
    // pattern indented — so a pattern that itself contains `error: ` (say
    // `error: [a`) makes a forward scan strip the prefix off the echo and
    // report the user's text back at them instead of the real cause.
    rendered
        .lines()
        .rev()
        .find_map(|line| line.trim().strip_prefix("error: "))
        .or_else(|| rendered.lines().find(|line| !line.trim().is_empty()))
        .unwrap_or_default()
        .trim()
        .to_string()
}

/// True when every char of `needle` appears in `haystack` in order (gaps
/// allowed). Both sides are expected to be lowercased already. A boolean is
/// enough — unlike k9s we do not rank fuzzy hits, so no scoring and no extra
/// dependency.
fn is_subsequence(needle: &[char], haystack: &str) -> bool {
    let mut remaining = needle.iter();
    let mut next = remaining.next();
    for candidate in haystack.chars() {
        match next {
            Some(&wanted) if wanted == candidate => next = remaining.next(),
            Some(_) => {}
            None => return true,
        }
    }
    next.is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_multi_line_regex_error_without_a_reason_line_is_reduced_to_one_line() {
        let error = regex::Error::Syntax("first line\nsecond line\nthird line".to_string());
        let reason = regex_reason(&error);
        assert_eq!(reason, "first line");
    }

    // The pattern is the point: these tests assert the *error*, so the literal
    // has to be invalid. `clippy::invalid_regex` const-evaluates it and rejects
    // it without knowing that. Hiding the literal from the lint would hide the
    // intent too, so the refusal is answered rather than dodged.
    #[allow(clippy::invalid_regex, reason = "the invalid pattern is the fixture")]
    #[test]
    fn a_regex_parse_error_keeps_only_its_reason_line() {
        let error = Regex::new("[unclosed").expect_err("pattern is invalid");
        let reason = regex_reason(&error);
        assert!(!reason.contains('\n'), "{reason:?}");
        assert!(reason.contains("character class"), "{reason:?}");
    }

    // Invalid on purpose, as above.
    #[allow(clippy::invalid_regex, reason = "the invalid pattern is the fixture")]
    #[test]
    fn a_pattern_that_embeds_the_error_sigil_still_reports_the_real_reason() {
        // The rendered block echoes the user's pattern indented above the
        // caret, so `error: [a` appears twice: once as their own text and once
        // as the reason prefix. Only the last one is the reason.
        let error = Regex::new("error: [a").expect_err("pattern is invalid");
        let reason = regex_reason(&error);
        assert_eq!(reason, "unclosed character class", "{reason:?}");
    }

    #[test]
    fn both_surfaces_report_the_real_reason_for_a_pattern_embedding_the_sigil() {
        use crate::gcp::filter::compile_filter;
        use crate::gcp::text_filter::{TextFilter, TextFilterError};

        let TextFilterError::InvalidRegex(log_message) =
            &TextFilter::parse("/error: [a/").expect_err("pattern is invalid");
        assert_eq!(log_message, "unclosed character class", "{log_message:?}");

        let table_message = compile_filter("error: [a")
            .expect_err("pattern is invalid")
            .message;
        assert_eq!(
            table_message, "invalid regex: unclosed character class",
            "{table_message:?}"
        );
    }

    #[test]
    fn an_empty_pattern_of_any_syntax_is_everything() {
        assert!(matches!(Matcher::substring(""), Matcher::Everything));
        assert!(matches!(Matcher::fuzzy(""), Matcher::Everything));
        assert!(matches!(
            Matcher::regex("").expect("empty regex is not compiled"),
            Matcher::Everything
        ));
    }

    #[test]
    fn a_predicate_inverts_the_result_over_the_whole_haystack_list() {
        let plain = Predicate::new(Matcher::substring("error"), false);
        let inverted = Predicate::new(Matcher::substring("error"), true);
        let haystacks = ["all quiet", "a big ERROR happened"];
        assert!(plain.matches_any(haystacks));
        assert!(!inverted.matches_any(haystacks));
        assert!(!plain.is_match_all());
        assert!(!inverted.is_match_all());
        assert!(inverted.is_inverted());
    }

    #[test]
    fn only_an_uninverted_empty_pattern_is_match_all() {
        assert!(Predicate::new(Matcher::Everything, false).is_match_all());
        assert!(!Predicate::new(Matcher::Everything, true).is_match_all());
    }
}
