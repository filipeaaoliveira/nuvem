//! The table filter grammar: one input, four modes selected by a prefix.
//!
//! This lives in the domain crate because both layers above it need the same
//! parse: the state layer evaluates the predicate over loaded rows, and the
//! service layer translates the same parsed query into a `gcloud --filter`
//! expression. Anywhere else and one of those layers would import from the
//! other.
//!
//! The module is pure — no I/O, no `gcloud`, no app state.
//!
//! An unprefixed input is a **regex**, and that is deliberately not what the
//! log grammar in [`crate::gcp::text_filter`] does with an unprefixed input
//! (there it is a literal substring). Both grammars compile down to the same
//! primitives in [`crate::gcp::matcher`], so it is tempting to give them one
//! default too — don't. A table filter is typed against short structured
//! values (names, zones, statuses) where `^api-.*-1$` earns its keep, while
//! the commonest log search is a bracketed level such as `[ERROR]`, which
//! under regex is a character class matching every line with an `E`, `R` or
//! `O` and never the brackets the user typed. The defaults differ because
//! the two kinds of haystack differ.

use crate::gcp::matcher::{Matcher, Predicate, regex_reason};
use crate::gcp::resource::ResourceServiceKind;

/// Which of the four filter behaviours an input selects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterMode {
    Regex,
    Fuzzy,
    Label,
    Inverse,
}

impl FilterMode {
    /// Every variant, for callers that walk the modes. Kept in sync by hand:
    /// the fixed length means adding a variant here is a deliberate edit, not
    /// something the compiler will demand.
    pub const ALL: [Self; 4] = [Self::Regex, Self::Fuzzy, Self::Label, Self::Inverse];

    /// Lowercase name shown in the filter UI chip.
    pub fn label(self) -> &'static str {
        match self {
            Self::Regex => "regex",
            Self::Fuzzy => "fuzzy",
            Self::Label => "label",
            Self::Inverse => "inverse",
        }
    }
}

/// One `key=value` pair of a label filter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelSelector {
    pub key: String,
    pub value: String,
}

/// A parsed, not-yet-compiled filter.
///
/// `None` covers both an empty input and a prefix whose remainder is empty:
/// the user is mid-typing and every row stays visible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilterQuery {
    None,
    Regex(String),
    Inverse(String),
    Fuzzy(String),
    Label(Vec<LabelSelector>),
}

/// A filter the user typed that cannot be understood. The message is
/// user-facing and rendered inline next to the filter input, so it is kept to
/// one line.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct FilterParseError {
    pub message: String,
}

impl FilterParseError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

const FUZZY_PREFIX: &str = "-f";
const LABEL_PREFIX: &str = "-l";
const INVERSE_PREFIX: &str = "!";

/// Splits trimmed input into its mode and the trimmed remainder after the
/// prefix. Shared by `parse_filter` and `filter_mode` so the two can never
/// disagree about what a half-typed input means.
fn split_prefix(trimmed: &str) -> (FilterMode, &str) {
    if let Some(rest) = trimmed.strip_prefix(FUZZY_PREFIX) {
        (FilterMode::Fuzzy, rest.trim())
    } else if let Some(rest) = trimmed.strip_prefix(LABEL_PREFIX) {
        (FilterMode::Label, rest.trim())
    } else if let Some(rest) = trimmed.strip_prefix(INVERSE_PREFIX) {
        (FilterMode::Inverse, rest.trim())
    } else {
        (FilterMode::Regex, trimmed)
    }
}

/// Parses the prefix grammar. `input` is trimmed first.
pub fn parse_filter(input: &str) -> Result<FilterQuery, FilterParseError> {
    let (mode, rest) = split_prefix(input.trim());
    if rest.is_empty() {
        return Ok(FilterQuery::None);
    }
    Ok(match mode {
        FilterMode::Regex => FilterQuery::Regex(rest.to_string()),
        FilterMode::Fuzzy => FilterQuery::Fuzzy(rest.to_string()),
        FilterMode::Inverse => FilterQuery::Inverse(rest.to_string()),
        FilterMode::Label => FilterQuery::Label(parse_label_selectors(rest)?),
    })
}

/// Splits `k=v,k=v` into selectors. Each pair splits on its *first* `=` so a
/// value may itself contain `=`.
fn parse_label_selectors(rest: &str) -> Result<Vec<LabelSelector>, FilterParseError> {
    rest.split(',')
        .map(|pair| {
            let (key, value) = pair.split_once('=').ok_or_else(|| {
                FilterParseError::new("malformed label selector: expected key=value")
            })?;
            let key = key.trim();
            let value = value.trim();
            if key.is_empty() {
                return Err(FilterParseError::new("malformed label selector: empty key"));
            }
            if value.is_empty() {
                return Err(FilterParseError::new(
                    "malformed label selector: empty value",
                ));
            }
            Ok(LabelSelector {
                key: key.to_string(),
                value: value.to_string(),
            })
        })
        .collect()
}

/// The mode of arbitrary text, including a half-typed draft. Never fails, so
/// the UI can label the mode while the user types. Empty input is `Regex`.
pub fn filter_mode(input: &str) -> FilterMode {
    split_prefix(input.trim()).0
}

/// A parsed filter with its regex compiled, ready to evaluate per row.
#[derive(Debug, Clone)]
pub enum CompiledFilter {
    None,
    Regex(Predicate),
    /// The same predicate as [`CompiledFilter::Regex`] with the inverse flag
    /// set — inversion is implemented once, in
    /// [`crate::gcp::matcher::Predicate`], so `!` cannot drift apart from the
    /// `!` of the log grammar.
    Inverse(Predicate),
    Fuzzy(Predicate),
    /// Carries no data: label mode is server-authoritative and never narrows
    /// client-side, because list rows carry no labels. The server's
    /// `--filter` pushdown does the narrowing; keeping every returned row is
    /// trivially safe under the never-drop-a-row invariant.
    Label,
}

/// Parses `input` and compiles any regex it contains.
pub fn compile_filter(input: &str) -> Result<CompiledFilter, FilterParseError> {
    Ok(match parse_filter(input)? {
        FilterQuery::None => CompiledFilter::None,
        FilterQuery::Regex(pattern) => {
            CompiledFilter::Regex(Predicate::new(compile_pattern(&pattern)?, false))
        }
        FilterQuery::Inverse(pattern) => {
            let predicate = Predicate::new(compile_pattern(&pattern)?, true);
            debug_assert!(
                predicate.is_inverted(),
                "CompiledFilter::Inverse must carry an inverted predicate: `matches` reads the \
                 inverse flag off the payload, not off the variant name"
            );
            CompiledFilter::Inverse(predicate)
        }
        FilterQuery::Fuzzy(needle) => {
            CompiledFilter::Fuzzy(Predicate::new(Matcher::fuzzy(&needle), false))
        }
        FilterQuery::Label(_) => CompiledFilter::Label,
    })
}

/// Compiles a user pattern case-insensitively.
fn compile_pattern(pattern: &str) -> Result<Matcher, FilterParseError> {
    Matcher::regex(pattern)
        .map_err(|error| FilterParseError::new(format!("invalid regex: {}", regex_reason(&error))))
}

impl CompiledFilter {
    /// `fields` are a row's searchable display values.
    pub fn matches(&self, fields: &[String]) -> bool {
        match self {
            Self::None | Self::Label => true,
            Self::Regex(predicate) | Self::Inverse(predicate) | Self::Fuzzy(predicate) => {
                predicate.matches_any(fields.iter().map(String::as_str))
            }
        }
    }
}

/// The raw gcloud JSON path holding user labels for a service, or `None` when
/// that service has no label pushdown. Cloud Run and Cloud Storage list output
/// exposes no user labels, so label filtering is unsupported there rather than
/// silently matching everything.
pub fn label_filter_field(kind: ResourceServiceKind) -> Option<&'static str> {
    match kind {
        ResourceServiceKind::ComputeInstances => Some("labels"),
        ResourceServiceKind::GkeClusters => Some("resourceLabels"),
        ResourceServiceKind::CloudSql => Some("settings.userLabels"),
        ResourceServiceKind::SecretManagerSecrets => Some("labels"),
        ResourceServiceKind::CloudFunctions => Some("labels"),
        ResourceServiceKind::PersistentDisks => Some("labels"),
        ResourceServiceKind::PubSubTopics => Some("labels"),
        ResourceServiceKind::PubSubSubscriptions => Some("labels"),
        // Neither a service account nor a policy binding carries user labels at
        // all — the IAM API has no field for them — so label filtering is
        // unsupported here rather than silently matching every row.
        //
        // A BigQuery dataset does carry user labels, but the listing comes
        // from the `bq` binary, which takes no server-side filter expression
        // at all. There is no clause to push down, so this is `None` for the
        // same end reason: better an unsupported message than a filter that
        // silently matches every dataset.
        //
        // No VPC resource carries user labels: the `compute/v1` networks,
        // subnetworks and firewalls resources have no `labels` field at all,
        // unlike their instances and disks siblings. Label filtering reports
        // unsupported rather than pushing a clause that would match every row
        // in the project.
        ResourceServiceKind::CloudRunServices
        | ResourceServiceKind::CloudStorage
        | ResourceServiceKind::BigQueryDatasets
        | ResourceServiceKind::IamServiceAccounts
        | ResourceServiceKind::IamPolicy
        | ResourceServiceKind::VpcNetworks
        | ResourceServiceKind::VpcSubnets
        | ResourceServiceKind::FirewallRules => None,
    }
}
