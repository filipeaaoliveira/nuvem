//! The log viewer's one-line search grammar.
//!
//! The grammar, applied strictly in this order:
//!
//! 1. A leading `!` sets the inverse flag and is stripped, along with any
//!    whitespace between it and the pattern, so `! /err/` is an inverted
//!    regex rather than an inverted substring search for `" /err/"`.
//! 2. What remains is a case-insensitive **regex** when it is delimited by `/`
//!    on both ends and is at least two characters long (`/x/`; bare `//` is an
//!    empty regex, which matches everything). An unparseable pattern is an
//!    [`TextFilterError::InvalidRegex`] carrying a renderable message — never a
//!    panic, because the input comes straight from a user's keystrokes.
//! 3. Otherwise a leading `~` marks a case-insensitive **fuzzy** subsequence:
//!    every pattern character appears in the haystack in order, not necessarily
//!    adjacent.
//! 4. Otherwise the whole input is a case-insensitive **substring**.
//!
//! Substring is deliberately the default, which is where this grammar diverges
//! from the k9s convention of defaulting to regex. The single most common log
//! search is a bracketed level such as `[ERROR]`, and under regex that is a
//! character class: it would silently match every line containing an `E`, `R`,
//! or `O` and would never match the brackets the user typed. Making the literal
//! reading the default and the regex opt-in explicit (`/…/`) means a typed
//! pattern can never quietly mean something other than what it looks like. `!`
//! keeps the same "inverse" meaning it has on every other surface.
//!
//! The grammar is decided in exactly one place, [`TextFilterSyntax::of`], and
//! exposed through two views of that one decision: the syntax view itself,
//! which names the mode without compiling anything (the header labels every
//! frame, so it must be free), and [`TextFilter::parse`], which takes the same
//! decision and compiles the matcher it selected. Neither re-inspects the
//! input, so they cannot come to disagree about what a pattern means.
//!
//! This module is the log surface's parser over the shared primitives, and
//! only that: it owns the syntax above and the log-side default. The
//! surface-neutral part — the matchers themselves, which know about strings and
//! nothing about logs or resources — is [`crate::gcp::matcher`].
//!
//! Everything below the parse is that module: this file only
//! decides which primitive an input selects, and the table grammar in
//! [`crate::gcp::filter`] makes the same kind of decision over the same
//! primitives with a different default. Sharing the primitives is not an
//! invitation to share the default — see the paragraph above.

use crate::gcp::matcher::{Matcher, Predicate, regex_reason};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextFilterKind {
    Substring,
    Fuzzy,
    Regex,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TextFilterError {
    #[error("invalid regex: {0}")]
    InvalidRegex(String),
}

/// A parsed filter: which syntax the user typed, plus the shared predicate
/// that syntax compiled to. `kind` exists only so the UI can name the mode —
/// an empty pattern of any syntax compiles to the same match-everything
/// predicate, and `kind` is what still distinguishes `//` from `~` from an
/// empty input.
#[derive(Debug, Clone)]
pub struct TextFilter {
    kind: TextFilterKind,
    predicate: Predicate,
}

/// What a pattern means *syntactically*, decided without compiling anything.
///
/// This is the whole of the grammar: the sigil walk in
/// [`TextFilterSyntax::of`] is the only place that decides which kind an input
/// selects, what its inverse flag is, and what needle is left once the sigils
/// are stripped. [`TextFilter::parse`] consumes exactly this decision and only
/// adds the compile step, so the syntax view and the compiled view cannot
/// disagree about what a pattern means — there is one parser, viewed twice.
///
/// The UI uses this view to label the search mode in the header without paying
/// for a regex compile per frame. It is a borrow of the input, so it costs
/// nothing but the walk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextFilterSyntax<'a> {
    kind: TextFilterKind,
    inverted: bool,
    /// The needle left after the sigils are stripped: the regex source for
    /// `/…/`, the subsequence for `~…`, otherwise the whole trimmed pattern.
    needle: &'a str,
}

impl<'a> TextFilterSyntax<'a> {
    /// Walks the grammar. The ordering below is load-bearing: `!` is stripped
    /// and the remainder re-trimmed first, then `/…/`, then `~`, then
    /// substring as the default.
    pub fn of(input: &'a str) -> Self {
        let trimmed = input.trim();
        // Re-trim after stripping `!`: the sigil detection below must see the
        // pattern the user typed, so `! /err/` is an inverted regex and not an
        // inverted substring search for a space followed by `/err/`.
        let (inverted, pattern) = match trimmed.strip_prefix('!') {
            Some(rest) => (true, rest.trim_start()),
            None => (false, trimmed),
        };

        // `/` is ASCII, so the byte-length and byte-slice arithmetic below is
        // safe for any UTF-8 pattern.
        if pattern.len() >= 2 && pattern.starts_with('/') && pattern.ends_with('/') {
            return Self {
                kind: TextFilterKind::Regex,
                inverted,
                needle: &pattern[1..pattern.len() - 1],
            };
        }

        if let Some(source) = pattern.strip_prefix('~') {
            return Self {
                kind: TextFilterKind::Fuzzy,
                inverted,
                needle: source,
            };
        }

        Self {
            kind: TextFilterKind::Substring,
            inverted,
            needle: pattern,
        }
    }

    /// Which syntax the input selected, decided without compiling anything.
    pub fn kind(&self) -> TextFilterKind {
        self.kind
    }

    /// Whether a leading `!` negated the pattern.
    pub fn is_inverted(&self) -> bool {
        self.inverted
    }

    /// True when this syntax accepts every haystack. An empty needle of any
    /// syntax compiles to [`Matcher::Everything`], so match-all is exactly an
    /// empty needle that is not inverted — a bare `!` inverts an empty pattern
    /// and therefore matches *nothing*.
    pub fn is_match_all(&self) -> bool {
        !self.inverted && self.needle.is_empty()
    }

    /// Short human-readable mode label for the UI hint, prefixed with `not `
    /// when inverted.
    pub fn describe(&self) -> String {
        describe(self.kind, self.inverted)
    }
}

/// The one rendering of a mode label, shared by both views of the parse.
fn describe(kind: TextFilterKind, inverted: bool) -> String {
    let label = match kind {
        TextFilterKind::Substring => "substring",
        TextFilterKind::Fuzzy => "fuzzy",
        TextFilterKind::Regex => "regex",
    };
    if inverted {
        format!("not {label}")
    } else {
        label.to_string()
    }
}

impl TextFilter {
    /// Parses the grammar. An empty or whitespace-only input yields a filter
    /// that matches everything.
    ///
    /// The syntax decision is not made here: it comes from
    /// [`TextFilterSyntax::of`], and this function only compiles the matcher
    /// that decision selected. That is what keeps the label the header shows
    /// and the filter the rows are matched against in permanent agreement.
    pub fn parse(input: &str) -> Result<TextFilter, TextFilterError> {
        let syntax = TextFilterSyntax::of(input);
        let matcher = match syntax.kind {
            TextFilterKind::Regex => Matcher::regex(syntax.needle)
                .map_err(|error| TextFilterError::InvalidRegex(regex_reason(&error)))?,
            TextFilterKind::Fuzzy => Matcher::fuzzy(syntax.needle),
            TextFilterKind::Substring => Matcher::substring(syntax.needle),
        };
        Ok(TextFilter {
            kind: syntax.kind,
            predicate: Predicate::new(matcher, syntax.inverted),
        })
    }

    /// True when any of `haystacks` matches, XOR-ed with the inverse flag. An
    /// empty haystack list never matches, so it yields the inverse flag itself.
    pub fn matches_any<'a>(&self, haystacks: impl IntoIterator<Item = &'a str>) -> bool {
        self.predicate.matches_any(haystacks)
    }

    /// True when the filter accepts every haystack and callers may skip
    /// filtering altogether. Inverting an empty pattern (`!` alone) matches
    /// *nothing*, so it is never match-all.
    pub fn is_match_all(&self) -> bool {
        self.predicate.is_match_all()
    }

    /// Which syntax this filter was parsed from — what the header labels, and
    /// the only thing left distinguishing `//` from `~` from an empty input
    /// once all three have compiled to the same predicate.
    pub fn kind(&self) -> TextFilterKind {
        self.kind
    }

    /// Whether a leading `!` negated the pattern; [`TextFilter::matches_any`]
    /// has already applied it.
    pub fn is_inverted(&self) -> bool {
        self.predicate.is_inverted()
    }

    /// Short human-readable mode label for the UI hint, prefixed with `not `
    /// when inverted.
    pub fn describe(&self) -> String {
        describe(self.kind, self.is_inverted())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(input: &str) -> TextFilter {
        TextFilter::parse(input).expect("pattern parses")
    }

    #[test]
    fn substring_is_the_default_kind_and_matches_case_insensitively() {
        let filter = parse("error");
        assert_eq!(filter.kind(), TextFilterKind::Substring);
        assert!(!filter.is_inverted());
        assert!(!filter.is_match_all());
        assert_eq!(filter.describe(), "substring");
        assert!(filter.matches_any(["a big ERROR happened"]));
        assert!(parse("ERROR").matches_any(["a big error happened"]));
    }

    #[test]
    fn substring_misses_when_no_haystack_contains_it() {
        let filter = parse("timeout");
        assert!(!filter.matches_any(["all good", "still fine"]));
    }

    #[test]
    fn bracketed_log_level_is_a_literal_substring_not_a_character_class() {
        // The motivating case for defaulting to substring: under regex,
        // `[ERROR]` is a character class and would never match the bracketed
        // text a log line actually contains.
        let filter = parse("[ERROR]");
        assert_eq!(filter.kind(), TextFilterKind::Substring);
        assert!(filter.matches_any(["2026-07-22T10:00:00Z [ERROR] disk full"]));
        assert!(!filter.matches_any(["2026-07-22T10:00:00Z ERROR disk full"]));
    }

    #[test]
    fn fuzzy_matches_a_case_insensitive_subsequence() {
        let filter = parse("~fb");
        assert_eq!(filter.kind(), TextFilterKind::Fuzzy);
        assert_eq!(filter.describe(), "fuzzy");
        assert!(filter.matches_any(["foobar"]));
        assert!(parse("~ERR").matches_any(["an error occurred"]));
    }

    #[test]
    fn fuzzy_misses_when_a_pattern_char_is_absent() {
        assert!(!parse("~fz").matches_any(["foobar"]));
    }

    #[test]
    fn fuzzy_misses_when_pattern_chars_are_out_of_order() {
        assert!(!parse("~bf").matches_any(["foobar"]));
    }

    #[test]
    fn slash_delimited_input_is_a_case_insensitive_regex() {
        let filter = parse("/err(or)?/");
        assert_eq!(filter.kind(), TextFilterKind::Regex);
        assert_eq!(filter.describe(), "regex");
        assert!(filter.matches_any(["an error occurred"]));
        assert!(!filter.matches_any(["all quiet"]));
        assert!(parse("/ERROR/").matches_any(["an error occurred"]));
    }

    #[test]
    fn regex_character_class_matches_where_the_substring_default_would_not() {
        let filter = parse("/[ERROR]/");
        assert_eq!(filter.kind(), TextFilterKind::Regex);
        assert!(filter.matches_any(["a plain E with no brackets"]));
    }

    #[test]
    fn invalid_regex_is_a_renderable_error_not_a_panic() {
        let error = TextFilter::parse("/[unclosed/").expect_err("pattern is invalid");
        let TextFilterError::InvalidRegex(message) = &error;
        assert!(!message.is_empty(), "message must be renderable");
        assert!(
            error.to_string().contains("regex"),
            "unexpected display: {error}"
        );
    }

    #[test]
    fn a_single_slash_is_a_substring_not_an_empty_regex() {
        let filter = parse("/");
        assert_eq!(filter.kind(), TextFilterKind::Substring);
        assert!(filter.matches_any(["projects/dev/logs"]));
    }

    #[test]
    fn empty_and_whitespace_only_input_matches_everything() {
        for input in ["", "   ", "\t"] {
            let filter = parse(input);
            assert!(filter.is_match_all(), "{input:?} should match all");
            assert!(!filter.is_inverted());
            assert!(filter.matches_any(["anything at all"]), "{input:?}");
        }
    }

    #[test]
    fn bare_slashes_are_an_empty_regex_that_matches_everything() {
        let filter = parse("//");
        assert_eq!(filter.kind(), TextFilterKind::Regex);
        assert!(filter.is_match_all());
        assert!(filter.matches_any(["anything at all"]));
    }

    #[test]
    fn a_bare_tilde_is_an_empty_fuzzy_pattern_that_matches_everything() {
        let filter = parse("~");
        assert_eq!(filter.kind(), TextFilterKind::Fuzzy);
        assert!(filter.is_match_all());
        assert!(filter.matches_any(["anything at all"]));
    }

    #[test]
    fn bang_alone_matches_nothing() {
        let filter = parse("!");
        assert!(filter.is_inverted());
        assert!(!filter.is_match_all());
        assert_eq!(filter.describe(), "not substring");
        assert!(!filter.matches_any(["anything at all"]));
    }

    #[test]
    fn bang_inverts_each_kind() {
        let substring = parse("!error");
        assert_eq!(substring.kind(), TextFilterKind::Substring);
        assert!(substring.is_inverted());
        assert_eq!(substring.describe(), "not substring");
        assert!(!substring.matches_any(["a big ERROR happened"]));
        assert!(substring.matches_any(["all quiet"]));

        let fuzzy = parse("!~fb");
        assert_eq!(fuzzy.kind(), TextFilterKind::Fuzzy);
        assert!(fuzzy.is_inverted());
        assert_eq!(fuzzy.describe(), "not fuzzy");
        assert!(!fuzzy.matches_any(["foobar"]));
        assert!(fuzzy.matches_any(["quiet"]));

        let regex = parse("!/err(or)?/");
        assert_eq!(regex.kind(), TextFilterKind::Regex);
        assert!(regex.is_inverted());
        assert_eq!(regex.describe(), "not regex");
        assert!(!regex.matches_any(["an error occurred"]));
        assert!(regex.matches_any(["all quiet"]));
    }

    #[test]
    fn a_space_after_the_bang_does_not_change_the_mode_or_the_pattern() {
        // The sigil detection must see the pattern the user typed, not a copy
        // with the separating space still glued to its front.
        let regex = parse("! /err/");
        assert_eq!(regex.kind(), TextFilterKind::Regex);
        assert!(regex.is_inverted());
        assert_eq!(regex.describe(), "not regex");
        assert!(!regex.matches_any(["an error occurred"]));
        assert!(regex.matches_any(["all quiet"]));

        let substring = parse("! error");
        assert_eq!(substring.kind(), TextFilterKind::Substring);
        assert!(substring.is_inverted());
        // Inverted `error`, not inverted `" error"`: a line whose only `error`
        // has no space before it must still be excluded.
        assert!(!substring.matches_any(["error: disk full"]));
        assert!(substring.matches_any(["all quiet"]));

        let fuzzy = parse("! ~fb");
        assert_eq!(fuzzy.kind(), TextFilterKind::Fuzzy);
        assert!(fuzzy.is_inverted());
        assert!(!fuzzy.matches_any(["foobar"]));
    }

    #[test]
    fn trailing_whitespace_is_not_part_of_a_substring_pattern() {
        // Typing a trailing space while composing must not require the log
        // line to contain one.
        let filter = parse("error ");
        assert_eq!(filter.kind(), TextFilterKind::Substring);
        assert!(filter.matches_any(["disk full: error"]));
    }

    #[test]
    fn invalid_regex_message_keeps_only_the_reason() {
        let error = TextFilter::parse("/[unclosed/").expect_err("pattern is invalid");
        let TextFilterError::InvalidRegex(message) = &error;
        assert!(
            !message.contains('\n') && !message.contains('\r'),
            "message must be a single line: {message:?}"
        );
        assert!(
            !message.contains('^'),
            "the caret line is meaningless once flattened: {message:?}"
        );
        assert!(
            !message.contains("regex parse error"),
            "the boilerplate header buries the reason: {message:?}"
        );
        assert!(
            message.contains("character class"),
            "message must name the reason: {message:?}"
        );
        assert!(
            message.len() <= 60,
            "message must fit a narrow header: {message:?}"
        );
    }

    #[test]
    fn any_matching_haystack_makes_the_whole_group_match() {
        let filter = parse("error");
        assert!(filter.matches_any(["all quiet", "a big ERROR happened"]));
        assert!(!parse("!error").matches_any(["all quiet", "a big ERROR happened"]));
    }

    #[test]
    fn an_empty_haystack_list_is_false_xor_the_inverse_flag() {
        let empty: [&str; 0] = [];
        assert!(!parse("error").matches_any(empty));
        assert!(!parse("").matches_any(empty));
        assert!(parse("!error").matches_any(empty));
        assert!(parse("!").matches_any(empty));
    }

    #[test]
    fn inverted_filters_are_never_match_all() {
        for input in ["!", "!error", "!~fb", "!//"] {
            assert!(!parse(input).is_match_all(), "{input:?}");
        }
    }

    // --- characterisation: repeated chars --------------------------------

    #[test]
    fn a_substring_needle_with_a_repeated_char_needs_them_adjacent() {
        let filter = parse("aa");
        assert!(filter.matches_any(["aabb"]));
        assert!(!filter.matches_any(["aba"]));
        assert!(!filter.matches_any(["a"]));
    }

    #[test]
    fn a_fuzzy_needle_with_a_repeated_char_needs_the_char_twice() {
        let filter = parse("~aa");
        assert!(!filter.matches_any(["api"]));
        assert!(filter.matches_any(["aabb"]));
        assert!(filter.matches_any(["a-b-a"]));
    }

    // --- characterisation: slash counting --------------------------------

    #[test]
    fn slash_counts_select_different_kinds() {
        // One slash is too short to delimit, two delimit an empty regex, and
        // three delimit a regex whose source is a literal slash.
        let one = parse("/");
        assert_eq!(one.kind(), TextFilterKind::Substring);
        assert!(one.matches_any(["projects/dev"]));
        assert!(!one.matches_any(["no separator here"]));

        let two = parse("//");
        assert_eq!(two.kind(), TextFilterKind::Regex);
        assert!(two.is_match_all());

        let three = parse("///");
        assert_eq!(three.kind(), TextFilterKind::Regex);
        assert!(!three.is_match_all());
        assert!(three.matches_any(["projects/dev"]));
        assert!(!three.matches_any(["no separator here"]));
    }

    // --- characterisation: inline regex flags ----------------------------

    #[test]
    fn an_inline_flag_in_the_pattern_overrides_the_builder_case_insensitivity() {
        let filter = parse("/(?-i)ERROR/");
        assert_eq!(filter.kind(), TextFilterKind::Regex);
        assert!(filter.matches_any(["a big ERROR happened"]));
        assert!(!filter.matches_any(["a big error happened"]));
    }

    // --- characterisation: case insensitivity in both directions ---------

    #[test]
    fn every_kind_is_case_insensitive_in_both_directions() {
        assert!(parse("error").matches_any(["A BIG ERROR"]));
        assert!(parse("ERROR").matches_any(["a big error"]));
        assert!(parse("~err").matches_any(["A BIG ERROR"]));
        assert!(parse("~ERR").matches_any(["a big error"]));
        assert!(parse("/err(or)?/").matches_any(["A BIG ERROR"]));
        assert!(parse("/ERR(OR)?/").matches_any(["a big error"]));
    }

    // --- characterisation: matches_any over a list -----------------------

    #[test]
    fn matches_any_scans_the_whole_list_for_every_kind() {
        let haystacks = ["all quiet", "a big ERROR happened", ""];
        assert!(parse("error").matches_any(haystacks));
        assert!(parse("~err").matches_any(haystacks));
        assert!(parse("/err(or)?/").matches_any(haystacks));
        assert!(!parse("timeout").matches_any(haystacks));
        assert!(!parse("~zq").matches_any(haystacks));
        assert!(!parse("/time(out)?/").matches_any(haystacks));
        // One hit anywhere in the list is enough to exclude an inverted filter.
        assert!(!parse("!error").matches_any(haystacks));
        assert!(!parse("!~err").matches_any(haystacks));
        assert!(!parse("!/err(or)?/").matches_any(haystacks));
        assert!(parse("!timeout").matches_any(haystacks));
    }

    // --- characterisation: describe --------------------------------------

    #[test]
    fn describe_covers_every_kind_and_inversion() {
        for (input, expected) in [
            ("error", "substring"),
            ("!error", "not substring"),
            ("", "substring"),
            ("!", "not substring"),
            ("~fb", "fuzzy"),
            ("!~fb", "not fuzzy"),
            ("~", "fuzzy"),
            ("!~", "not fuzzy"),
            ("/err/", "regex"),
            ("!/err/", "not regex"),
            ("//", "regex"),
            ("!//", "not regex"),
        ] {
            assert_eq!(parse(input).describe(), expected, "input {input:?}");
        }
    }

    // --- characterisation: non-ASCII -------------------------------------

    #[test]
    fn non_ascii_substrings_match_case_insensitively() {
        assert!(parse("café").matches_any(["le CAFÉ est ouvert"]));
        assert!(parse("CAFÉ").matches_any(["le café est ouvert"]));
        assert!(!parse("café").matches_any(["le the est ouvert"]));
    }

    #[test]
    fn non_ascii_fuzzy_needles_match_case_insensitively() {
        assert!(parse("~cfé").matches_any(["le CAFÉ est ouvert"]));
        assert!(parse("~CFÉ").matches_any(["le café est ouvert"]));
        assert!(!parse("~éc").matches_any(["le café est ouvert"]));
    }

    #[test]
    fn non_ascii_regex_delimiters_do_not_split_a_multibyte_char() {
        // `/` is ASCII so the byte slicing around it is UTF-8 safe; these must
        // parse and match rather than panic.
        let filter = parse("/caf./");
        assert_eq!(filter.kind(), TextFilterKind::Regex);
        assert!(filter.matches_any(["le café est ouvert"]));

        let accented = parse("/é/");
        assert_eq!(accented.kind(), TextFilterKind::Regex);
        assert!(accented.matches_any(["le café est ouvert"]));
        assert!(accented.matches_any(["le CAFÉ est ouvert"]));
        assert!(!accented.matches_any(["le the est ouvert"]));
    }

    #[test]
    fn a_bare_multibyte_char_is_a_substring_despite_its_byte_length() {
        // Two bytes but one char, so `len() >= 2` alone would admit it to the
        // regex branch; what actually keeps it a substring is `starts_with('/')`.
        let filter = parse("é");
        assert_eq!(filter.kind(), TextFilterKind::Substring);
        assert!(filter.matches_any(["le café"]));
    }

    #[test]
    fn a_needle_char_that_lowercases_to_two_chars_needs_both_in_the_haystack() {
        // `İ` lowercases to `i` plus a combining dot, so the one char the user
        // typed becomes two the haystack must supply.
        assert!(parse("İ").matches_any(["İstanbul"]));
        assert!(!parse("İ").matches_any(["Istanbul"]));
        assert!(parse("~İ").matches_any(["İstanbul"]));
        assert!(!parse("~İ").matches_any(["Istanbul"]));
    }

    // --- characterisation: sigils inside the pattern ---------------------

    #[test]
    fn a_sigil_inside_the_pattern_stays_literal() {
        let bang = parse("a!b");
        assert_eq!(bang.kind(), TextFilterKind::Substring);
        assert!(!bang.is_inverted());
        assert!(bang.matches_any(["xa!by"]));

        let tilde = parse("a~b");
        assert_eq!(tilde.kind(), TextFilterKind::Substring);
        assert!(tilde.matches_any(["xa~by"]));

        // A trailing-only slash cannot delimit either.
        let trailing = parse("dev/");
        assert_eq!(trailing.kind(), TextFilterKind::Substring);
        assert!(trailing.matches_any(["projects/dev/logs"]));
    }

    // --- the syntax view agrees with the compiled view -------------------

    #[test]
    fn the_syntax_view_agrees_with_the_compiled_parse_for_every_kind_and_inversion() {
        for input in [
            "error", "!error", "", "!", "   ", "!   ", "\t", "~fb", "!~fb", "~", "!~", "/err/",
            "!/err/", "//", "!//", "///", "!///", "/", "! /err/", "! error", "! ~fb",
            // Remainders that embed a sigil: the syntax view must strip
            // exactly the sigils `parse` strips and no more.
            "a!b", "!a!b", "!!error", "a~b", "dev/", "/a~b/", "!/a!b/",
        ] {
            let compiled = parse(input);
            let syntax = TextFilterSyntax::of(input);
            assert_eq!(syntax.kind(), compiled.kind(), "kind for {input:?}");
            assert_eq!(
                syntax.is_inverted(),
                compiled.is_inverted(),
                "inverted for {input:?}"
            );
            assert_eq!(
                syntax.is_match_all(),
                compiled.is_match_all(),
                "match-all for {input:?}"
            );
            assert_eq!(
                syntax.describe(),
                compiled.describe(),
                "describe for {input:?}"
            );
        }
    }

    #[test]
    fn the_syntax_view_still_reports_regex_for_a_pattern_that_does_not_compile() {
        // The one place the two views legitimately differ: the syntax view
        // never compiles, so it cannot know the regex is invalid. It reports
        // the syntax the user typed while `parse` errors — which is why the
        // header suppresses the mode label whenever it has a parse error.
        let syntax = TextFilterSyntax::of("/[a/");
        assert_eq!(syntax.kind(), TextFilterKind::Regex);
        assert!(!syntax.is_inverted());
        assert!(!syntax.is_match_all());
        assert_eq!(syntax.describe(), "regex");
        assert!(TextFilter::parse("/[a/").is_err());

        let inverted = TextFilterSyntax::of("!/[a/");
        assert_eq!(inverted.kind(), TextFilterKind::Regex);
        assert!(inverted.is_inverted());
        assert_eq!(inverted.describe(), "not regex");
        assert!(TextFilter::parse("!/[a/").is_err());
    }

    #[test]
    fn only_the_first_bang_inverts() {
        let filter = parse("!!error");
        assert!(filter.is_inverted());
        assert_eq!(filter.kind(), TextFilterKind::Substring);
        // The second `!` is part of the needle.
        assert!(!filter.matches_any(["a big !error happened"]));
        assert!(filter.matches_any(["a big error happened"]));
    }
}
