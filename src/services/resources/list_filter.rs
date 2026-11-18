//! Shared helper for pushing the resource-list `/` filter down to gcloud as a
//! server-side `--filter` expression.
//!
//! **Never-drop-a-row invariant.** The client filter is a case-insensitive
//! match across several *display* fields — a regex partial match in the
//! grammar's default mode, a literal substring in the Phase 1 helpers below
//! that still take a bare term — while the server searches *raw* gcloud JSON.
//! A pushed `--filter` reloads a server-narrowed set that *replaces* the list,
//! so the pushdown must be a strict SUPERSET of the client match, or matching
//! rows silently vanish. Two rules guarantee that:
//!
//! 1. **Only push verbatim-superset fields** — raw JSON fields whose value
//!    contains the displayed value as a substring (name, URL-backed
//!    zone/machineType/location, raw status/state, IP strings, ...). A regex
//!    partial match (`~ "(?i)term"`) on such a field is a genuine per-field
//!    superset of the literal client substring match.
//! 2. **Gate out terms that could match a transformed or derived display
//!    label with no faithful raw counterpart** — e.g. compute status labels
//!    (`Running`/`Stopped`/...), GKE endpoint visibility (`Public`/`Private`),
//!    Cloud SQL engine (`PostgreSQL`) or availability (`HA (regional)`). If
//!    the lowercased term is a substring of any such opaque label, the term
//!    might match a row via that derived field even though the raw pushed
//!    fields don't contain it, so the pushdown returns `None` and the caller
//!    falls back to full client-side narrowing instead of risking a dropped
//!    row. [`pushdown_expression`] applies this gate in its pattern-aware
//!    form — see `pattern_matches_opaque_label`, which asks whether the
//!    compiled pattern *matches* an opaque label rather than whether the term
//!    is a substring of one.
//!
//! **The premise both rules rest on.** Rules 1 and 2 only partition the
//! display values if there is nothing *outside* the partition. So, for every
//! service on this path, each value in that service's `searchable_fields`
//! must be either
//!
//! - a verbatim substring of one of the raw JSON fields pushed in its
//!   `FILTER_FIELDS`, or
//! - literally enumerated, lowercased, in its `OPAQUE_LABELS`.
//!
//! A value that is neither is a hole: a term can match it client-side while
//! the gate stays silent and the pushed expression matches no raw field, and
//! the row vanishes. Adding a service — or a searchable field to an existing
//! one — means checking every value against those two options. Watch in
//! particular for values that are *rendered* rather than stored: a
//! `format!("{:?}", …)` of an enum with a data-carrying variant emits a
//! `Variant("raw")` shell whose punctuation is in no raw field and in no
//! opaque label (this is why compute, GKE, and Cloud SQL push the status
//! *display label* plus, separately, the raw string carried by `Unknown`). A
//! regex term can cross such punctuation with `.` even when a literal
//! substring could not.
//!
//! Non-expressible terms (see [`is_server_expressible`]) also fall back to
//! `None`. Mirrors the spirit of Storage's `is_safe_remote_prefix` gate.
//!
//! **The multi-mode grammar.** [`pushdown_expression`] is the entry point for
//! an applied filter written in the prefix grammar
//! (`nuvem_domain::gcp::filter`). Only two of the four modes reach the server:
//!
//! - *regex* pushes the same `~ "(?i)<term>"` clauses as before. This stays a
//!   superset even though the client now treats the term as a pattern: the
//!   [`is_server_expressible`] character gate admits only ASCII alphanumerics
//!   and `.` `_` `-`, so the sole surviving metacharacter is `.`, which is
//!   position-independent — anchors, quantifiers, alternation, and character
//!   classes are all rejected. An admitted pattern therefore matches a display
//!   value only if it also matches, at the corresponding offset, any raw value
//!   that contains that display value as a substring, which is exactly what
//!   rule 1 above guarantees for the pushed fields.
//! - *label* pushes `field.key = "value"`. Label mode is server-authoritative:
//!   `CompiledFilter::Label` keeps every row unconditionally — it answers
//!   `true` whatever the row holds — so the client-side predicate cannot drop
//!   one, and all narrowing lives in the pushed expression. That is one rule
//!   for every service rather than a per-service one: most list row structs
//!   carry no labels to match against anyway, but `Secret` does, and the
//!   predicate ignores them just the same. A service whose pushdown is missing
//!   therefore does not fall back to client-side narrowing in this mode; it
//!   narrows not at all. Any doubt about the key, the value, or whether the
//!   service exposes labels at all resolves to `None`, which degrades to "no
//!   narrowing" rather than to a wrong row set.
//!
//! *fuzzy* (subsequence matching) and *inverse* (negation over all display
//! fields, including the derived ones) have no expression that is provably a
//! superset, so they are never pushed.
//!
//! Pure module: no I/O, no dependency on the gcloud runner or app state.

use nuvem_domain::gcp::filter::{
    CompiledFilter, FilterQuery, LabelSelector, compile_filter, parse_filter,
};

/// Upper bound on rows requested from a resource `list` command; caps the
/// download/parse cost of a poll while the client still narrows further.
pub const RESOURCE_LIST_LIMIT: usize = 500;

/// Whether a native listing has taken all the rows the cap allows.
///
/// Every native lister drains a paginator and stops when this says so. The
/// decision is one function rather than a comparison repeated at a dozen call
/// sites, for the reason `label_mode_needs_gcloud` is: a comparison inside a
/// network body cannot be tested -- a listing reaches the API, and the
/// harnesses hold no clients -- so each copy was a place where `>=` could
/// become `<` and no test would notice. Here it is a pure function with the
/// boundary pinned on both sides.
///
/// The `cap` argument is not always [`RESOURCE_LIST_LIMIT`]: the log reads
/// carry the viewer's own buffer size, which is chosen per call.
pub fn rows_are_capped(collected: usize, cap: usize) -> bool {
    collected >= cap
}

/// Returns `true` when the trimmed `term` is non-empty and every character is
/// ASCII alphanumeric or one of `.` `_` `-`.
///
/// Rejects whitespace, quotes, slashes, regex/glob metacharacters, and
/// non-ASCII characters, since those cannot be embedded safely in the
/// `~ "(?i)<term>"` regex clause built by [`server_filter_expression`].
pub fn is_server_expressible(term: &str) -> bool {
    let trimmed = term.trim();
    !trimmed.is_empty()
        && trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// Builds a server-side `--filter` expression that matches `term` as a
/// case-insensitive regex partial match against any of `fields`, unless the
/// term could also match an opaque transformed/derived display label.
///
/// Trims `term` first. Returns `None` if:
/// - `fields` is empty, or
/// - the trimmed term is not [`is_server_expressible`], or
/// - the lowercased trimmed term is a substring of any entry in
///   `opaque_labels` (each entry is expected to already be lowercase) — this
///   is the never-drop-a-row gate: such a term might match a display field
///   that is transformed/derived from raw JSON rather than verbatim-present
///   in `fields`, so pushing the filter down could drop a row the
///   client-side filter would have kept. Pass `&[]` to disable the gate.
///
/// Otherwise returns `Some`, joining with `" OR "` one clause per field of
/// the form `field ~ "(?i)<term>"`. This regex partial match is a superset
/// of the client's literal substring match on that field, so the caller can
/// safely re-apply the exact client filter to the (superset) result without
/// losing rows.
pub fn server_filter_expression(
    term: &str,
    fields: &[&str],
    opaque_labels: &[&str],
) -> Option<String> {
    let trimmed = term.trim();
    if fields.is_empty() || !is_server_expressible(trimmed) {
        return None;
    }

    let lowered = trimmed.to_lowercase();
    if opaque_labels.iter().any(|label| label.contains(&lowered)) {
        return None;
    }

    Some(
        fields
            .iter()
            .map(|field| format!(r#"{field} ~ "(?i){trimmed}""#))
            .collect::<Vec<_>>()
            .join(" OR "),
    )
}

/// Whether an applied filter can only be honoured by the server-side label
/// expression, so the read must go through `gcloud` even where a native client
/// is available.
///
/// Label mode narrows nothing on the client: almost no list row carries labels,
/// and `CompiledFilter::Label` keeps every row on purpose. The pushed
/// `labels.k = "v"` expression is the whole mechanism, and the native clients
/// push no filter, so a label-mode read served natively would show the whole
/// project under a `[label]` chip that says otherwise. Every other mode narrows
/// on the client and does not care which backend filled the rows.
///
/// An empty or malformed selector is not label mode: the client reports the
/// parse error inline and keeps every row, on either backend.
pub fn label_mode_needs_gcloud(applied: Option<&str>) -> bool {
    matches!(
        applied.and_then(|text| parse_filter(text).ok()),
        Some(FilterQuery::Label(selectors)) if !selectors.is_empty()
    )
}

/// The native clients a list read may use, or `None` when the read must go
/// through gcloud.
///
/// `None` for two reasons that look the same to the caller: no native clients
/// are configured (the test harnesses), or the applied filter is label mode,
/// which only the pushed gcloud expression honours. Demo mode reaches the
/// second of those, not the first: it holds fixture-backed clients like any
/// other session, and a label-mode read there falls back to the demo runner's
/// argv the same way a live one falls back to gcloud. Deciding here,
/// once, is what keeps every list helper free of a negation nothing can test:
/// the helpers ask this and branch, and this is pinned.
pub fn native_clients_for<'a, C>(clients: Option<&'a C>, applied: Option<&str>) -> Option<&'a C> {
    clients.filter(|_| !label_mode_needs_gcloud(applied))
}

/// Builds the `--filter` expression for an applied multi-mode filter, or
/// `None` when the filter must be narrowed client-side instead.
///
/// `applied` is the raw filter input in the prefix grammar; `fields` and
/// `opaque_labels` are the calling service's pushdown constants, and
/// `label_field` is the raw JSON path holding that service's user labels
/// (`nuvem_domain::gcp::filter::label_filter_field`), or `None` when the
/// service exposes none.
///
/// Unparsable input, an empty filter, fuzzy mode, and inverse mode all return
/// `None` — see the module docs for why each is not a provable superset.
///
/// In the regex arm only the first gate is load-bearing:
/// `pattern_matches_opaque_label` asks the client's own compiled predicate,
/// which subsumes the weaker substring gate that
/// [`server_filter_expression`] re-applies to the same `opaque_labels`. The
/// second pass is kept because `server_filter_expression` stays a standalone
/// public helper with its own Phase 1 contract, not because it catches
/// anything the first misses.
pub fn pushdown_expression(
    applied: &str,
    fields: &[&str],
    opaque_labels: &[&str],
    label_field: Option<&str>,
) -> Option<String> {
    match parse_filter(applied).ok()? {
        FilterQuery::None | FilterQuery::Fuzzy(_) | FilterQuery::Inverse(_) => None,
        FilterQuery::Regex(term) => {
            if pattern_matches_opaque_label(applied, opaque_labels) {
                return None;
            }
            server_filter_expression(&term, fields, opaque_labels)
        }
        FilterQuery::Label(selectors) => label_expression(&selectors, label_field?),
    }
}

/// Whether the regex the *client* compiles for `applied` matches any opaque
/// display label — the never-drop-a-row gate in its pattern-aware form.
///
/// Phase 1 asked whether the lowercased term was a *substring* of an opaque
/// label, which is exact for a literal but wrong for a pattern: `r.n` is no
/// substring of `running`, yet it matches the `Running` status cell, so
/// pushing it down would drop that row. Re-using
/// `nuvem_domain::gcp::filter::compile_filter` here means the gate asks the
/// client's own predicate, so the two can never disagree about what the
/// pattern matches. A pattern that does not compile is treated as gated: the
/// caller has nothing safe to push.
fn pattern_matches_opaque_label(applied: &str, opaque_labels: &[&str]) -> bool {
    let Ok(compiled) = compile_filter(applied) else {
        return true;
    };
    // `applied` parsed as regex mode, so this is always `CompiledFilter::Regex`
    // — but matching on the variant keeps the gate from silently degrading
    // into `CompiledFilter::None`'s match-everything behaviour.
    let CompiledFilter::Regex(_) = compiled else {
        return true;
    };
    opaque_labels
        .iter()
        .any(|label| compiled.matches(&[(*label).to_string()]))
}

/// Whether one `key=value` selector can be emitted as a gcloud clause.
///
/// Both sides must be [`is_server_expressible`] — the character gate keeps the
/// quoted literals free of quotes, whitespace, and expression syntax, so the
/// clause cannot be broken out of.
///
/// Keys are additionally rejected when they contain `.`, which
/// [`is_server_expressible`] admits: a key is interpolated *unquoted* after
/// the label field, so `a.b` would emit `labels.a.b = "c"` and gcloud would
/// read it as a nested field path. GCP label keys cannot contain `.` at all,
/// so such a selector is always a nonsense query — better to narrow
/// client-side than to load an empty table. Values keep `.`: they sit inside
/// the quoted operand, where a dot is inert.
///
/// Public because the state layer reports the same rejection as an inline
/// error (`ResourcesState::filter_error`); sharing the predicate is what keeps
/// that message and this pushdown's `None` from ever disagreeing.
pub fn is_expressible_label_selector(selector: &LabelSelector) -> bool {
    is_server_expressible(&selector.key)
        && !selector.key.contains('.')
        && is_server_expressible(&selector.value)
}

/// Joins one `field.key = "value"` clause per selector with `" AND "`.
/// Keys and values arrive already trimmed from the grammar.
///
/// Returns `None` unless there is at least one selector and every one of them
/// is [`is_expressible_label_selector`].
fn label_expression(selectors: &[LabelSelector], label_field: &str) -> Option<String> {
    if selectors.is_empty() || !selectors.iter().all(is_expressible_label_selector) {
        return None;
    }

    Some(
        selectors
            .iter()
            .map(|selector| {
                format!(
                    r#"{label_field}.{key} = "{value}""#,
                    key = selector.key,
                    value = selector.value
                )
            })
            .collect::<Vec<_>>()
            .join(" AND "),
    )
}
