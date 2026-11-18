//! Why a gcloud call failed, in terms the app can act on.
//!
//! Raw stderr is classified once into a [`GcloudErrorKind`], which answers both
//! whether retrying is worth anything and what to tell the operator to do. The
//! text itself is redacted and length-bounded before it is carried anywhere: a
//! failure message is the one place tokens and unasked-for resource names
//! arrive and get rendered verbatim.

use std::borrow::Cow;

use serde::{Deserialize, Serialize};

/// Coarse classification of a gcloud failure, used to explain degraded services.
///
/// Covers the full issue #31 taxonomy: auth, permission, availability, quota,
/// network, parsing, and input errors, each with a retryability and a
/// user-facing remediation hint.
///
/// Deliberately data-free. A missing `bq` and a missing `gcloud` are the same
/// *kind* of failure, and this type is what the app compares and stores to say
/// so: it is `Copy`, it is `Serialize`/`Deserialize`, and two values of it are
/// equal exactly when they classify the same. Giving `MissingExecutable` a
/// payload would cost all three — `Copy` survives only for a `Copy` payload,
/// the serialized form changes shape, and every site that compares two kinds
/// would start distinguishing failures that classification exists to unify.
///
/// The binary's name is not classification, it is the one word the remediation
/// sentence needs, so it rides on the carriers — [`DiagnosticProblem`] and
/// [`GcloudDiagnostic`] — beside the kind. See
/// [`GcloudErrorKind::label`] for what this costs a caller holding only a kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GcloudErrorKind {
    PermissionDenied,
    ApiDisabled,
    InvalidQuery,
    Quota,
    Network,
    Parse,
    MissingProject,
    Unknown,
    // --- issue #31 additive block (append only; do not reorder) ---
    MissingExecutable,
    Unauthenticated,
    InvalidInput,
    Cancelled,
}

impl GcloudErrorKind {
    /// The kind's own words, which can only say what the kind knows.
    ///
    /// `MissingExecutable` names no binary here on purpose. nuvem spawns more
    /// than one CLI (`gcloud` and `bq`), and this label used to read "missing
    /// gcloud CLI" for all of them — so a missing `bq` sent the reader off to
    /// reinstall a binary that was already installed. A caller holding only a
    /// kind cannot know which one it was; it gets wording that is vague but
    /// never wrong. Callers holding a carrier get the binary named: see
    /// [`DiagnosticProblem::label`] and [`GcloudDiagnostic::label`].
    pub fn label(self) -> &'static str {
        match self {
            Self::PermissionDenied => "permission denied",
            Self::ApiDisabled => "API disabled",
            Self::InvalidQuery => "invalid query",
            Self::Quota => "quota",
            Self::Network => "network",
            Self::Parse => "parse error",
            Self::MissingProject => "missing project",
            Self::Unknown => "unknown error",
            Self::MissingExecutable => "missing CLI",
            Self::Unauthenticated => "not authenticated",
            Self::InvalidInput => "invalid input",
            Self::Cancelled => "cancelled",
        }
    }
}

/// Whether a failure of this kind is safe to auto-retry in the background.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Retryability {
    /// Transient — safe for the poll ticker to retry (network blips, quota, unknown).
    Transient,
    /// Persistent — a config/auth/logic error; auto-retrying just hammers gcloud.
    Persistent,
}

impl GcloudErrorKind {
    /// Whether the background poll may retry a failure of this kind on its
    /// own, or would only hammer gcloud with a request that cannot succeed
    /// until the operator changes something.
    pub fn retryability(self) -> Retryability {
        match self {
            Self::Network | Self::Quota | Self::Unknown => Retryability::Transient,
            Self::PermissionDenied
            | Self::ApiDisabled
            | Self::InvalidQuery
            | Self::MissingProject
            | Self::Parse
            | Self::MissingExecutable
            | Self::Unauthenticated
            | Self::InvalidInput
            | Self::Cancelled => Retryability::Persistent,
        }
    }

    /// A concise, imperative remediation hint for this failure kind.
    pub fn user_action(self) -> &'static str {
        match self {
            Self::PermissionDenied => "Check IAM roles for this account and project",
            Self::ApiDisabled => "Enable the required API in the Cloud Console",
            Self::InvalidQuery => "Fix the query filter and retry",
            Self::Quota => "Wait and retry, or request a quota increase",
            Self::Network => "Check your network connection and retry",
            Self::Parse => "Update nuvem — gcloud output did not match the expected schema",
            Self::MissingProject => "Set an active project on this configuration",
            Self::Unknown => "Retry, or copy the details to investigate",
            Self::MissingExecutable => "Install the missing CLI and ensure it is on PATH",
            Self::Unauthenticated => "Run `gcloud auth login` to reauthenticate",
            Self::InvalidInput => "Check the command arguments and try again",
            Self::Cancelled => "The operation was cancelled; retry when ready",
        }
    }
}

/// Classifies gcloud stderr text into a coarse error kind. Case-insensitive
/// substring matching, most-specific categories first. Never classifies `Parse`
/// (parse failures are known from the error variant, not from stderr).
pub fn classify_gcloud_stderr(text: &str) -> GcloudErrorKind {
    let haystack = text.to_lowercase();
    let has = |needle: &str| haystack.contains(needle);

    if has("invalid_grant")
        || has("reauth")
        || has("gcloud auth login")
        || has("auth application-default login")
        || has("do not currently have an active account")
        || has("credentials are invalid")
        || has("unauthenticated")
        || has("(401)")
    {
        return GcloudErrorKind::Unauthenticated;
    }
    // Deliberately ABOVE the permission branch, and deliberately narrower
    // than the ApiDisabled block further down. Do not merge the two, and do
    // not move this below `PermissionDenied` to "simplify" the ordering.
    //
    // gcloud reports a disabled API with the gRPC code PERMISSION_DENIED on
    // the ERROR line, naming SERVICE_DISABLED only in the trailing ErrorInfo
    // block. The two signals therefore arrive in the same message, and
    // whichever arm is tested first decides. Tested permission-first, an API
    // that is merely switched off is reported as a permissions problem and
    // the operator is sent to audit IAM roles that are already correct.
    //
    // Only needles that name a *service* are hoisted. The loose "is disabled"
    // stays in the block below, on purpose: a disabled *principal* ("the
    // service account ... is disabled") is a real permission error wearing
    // the same two words, and hoisting them would tell that operator to
    // enable an API — the same wrong advice, pointing the other way. Below
    // the permission branch, "is disabled" still classifies text that
    // carries no permission signal at all.
    if has("service_disabled")
        || has("api_disabled")
        || has("accessnotconfigured")
        || has("has not been used in project")
        || has("enable it by visiting")
    {
        return GcloudErrorKind::ApiDisabled;
    }
    if has("permission denied")
        || has("permission_denied")
        || has("does not have permission")
        || has("forbidden")
        || has("(403)")
        || has(" 403")
    {
        return GcloudErrorKind::PermissionDenied;
    }
    // The ambiguous half of the same category, and the reason this is two
    // blocks rather than one. Reached only when nothing above matched, so a
    // message that pairs these words with a permission signal has already
    // been classified `PermissionDenied` — which is what "the service account
    // ... is disabled" needs. What is left here is text that says a service
    // is off and says nothing about permissions, and that is `ApiDisabled`.
    //
    // The needles the block above already returns on are deliberately absent:
    // repeating them here would be unreachable, and would read as a safety
    // net that does not exist.
    if has("has not been used") || has("is disabled") {
        return GcloudErrorKind::ApiDisabled;
    }
    if has("quota") || has("resource_exhausted") || has("rate limit") || has("(429)") {
        return GcloudErrorKind::Quota;
    }
    if has("could not reach")
        || has("network")
        || has("connection")
        || has("timed out")
        || has("timeout")
        || has("unreachable")
        || has("getaddrinfo")
        || has("dns")
    {
        return GcloudErrorKind::Network;
    }
    // Filter/query-specific INVALID_ARGUMENT (Cloud Logging, issue #30) is
    // classified before the generic invalid-input category below; bare
    // "invalid argument"/"(400)" without filter context falls through to
    // InvalidInput.
    if has("invalid_argument") || has("invalid filter") || has("unparseable filter") {
        return GcloudErrorKind::InvalidQuery;
    }
    if has("[project]") || has("no project") || has("project id must be specified") {
        return GcloudErrorKind::MissingProject;
    }
    if has("invalid argument")
        || has("invalid_argument")
        || has("unrecognized arguments")
        || has("invalid value")
        || has("invalid choice")
        || has("(400)")
    {
        return GcloudErrorKind::InvalidInput;
    }
    GcloudErrorKind::Unknown
}

/// Detects Cloud Storage "requester pays" access failures in gcloud stderr
/// (e.g. `userProjectMissing`, "Bucket is requester pays bucket but no user
/// project provided"). Kept deliberately separate from [`GcloudErrorKind`]:
/// requester-pays is a Cloud-Storage-specific access condition rather than a
/// coarse gcloud failure category, so callers that care (the storage
/// browser) check this alongside `classify_gcloud_stderr` instead of a new
/// enum variant.
pub fn stderr_indicates_requester_pays(text: &str) -> bool {
    let haystack = text.to_lowercase();
    haystack.contains("requester pays")
        || haystack.contains("requester-pays")
        || haystack.contains("userprojectmissing")
        || haystack.contains("user project missing")
}

/// A classified problem paired with its original human-readable message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticProblem {
    pub kind: GcloudErrorKind,
    pub message: String,
    /// Which binary was missing, for a failure that knows.
    pub missing_binary: Option<&'static str>,
}

/// The label for `kind`, naming `missing_binary` when the carrier knows which
/// binary it was.
///
/// `'static` rather than a `String` is the constraint that matters: the only
/// values it can hold are this repo's own literals (`CommandProgram::binary_name`
/// on the src side), never a user-configured binary path. That is why it is
/// interpolated into a display string without going through
/// [`sanitize_detail`], unlike every other text on these carriers.
pub fn label_naming_binary(
    kind: GcloudErrorKind,
    missing_binary: Option<&'static str>,
) -> Cow<'static, str> {
    match (kind, missing_binary) {
        (GcloudErrorKind::MissingExecutable, Some(binary)) => {
            Cow::Owned(format!("missing {binary} CLI"))
        }
        _ => Cow::Borrowed(kind.label()),
    }
}

/// The remediation hint for `kind`, naming `missing_binary` when known. See
/// [`label_naming_binary`] for why the name needs no sanitization.
pub fn user_action_naming_binary(
    kind: GcloudErrorKind,
    missing_binary: Option<&'static str>,
) -> Cow<'static, str> {
    match (kind, missing_binary) {
        (GcloudErrorKind::MissingExecutable, Some(binary)) => {
            Cow::Owned(format!("Install the {binary} CLI and ensure it is on PATH"))
        }
        _ => Cow::Borrowed(kind.user_action()),
    }
}

impl DiagnosticProblem {
    pub fn new(kind: GcloudErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            missing_binary: None,
        }
    }

    /// A `MissingExecutable` problem that names the binary that was missing.
    ///
    /// `binary` is a static literal from this repo — see
    /// [`label_naming_binary`].
    pub fn missing_executable(binary: &'static str, message: impl Into<String>) -> Self {
        Self {
            kind: GcloudErrorKind::MissingExecutable,
            message: message.into(),
            missing_binary: Some(binary),
        }
    }

    /// Carries a known missing binary onto a problem built by [`new`](Self::new).
    #[must_use]
    pub fn naming_missing_binary(mut self, binary: Option<&'static str>) -> Self {
        self.missing_binary = binary;
        self
    }

    /// The failure category in words, naming the missing binary when this
    /// problem knows it — as opposed to [`GcloudErrorKind::label`], which
    /// cannot.
    pub fn label(&self) -> Cow<'static, str> {
        label_naming_binary(self.kind, self.missing_binary)
    }

    /// The remediation hint, naming the missing binary when known.
    pub fn user_action(&self) -> Cow<'static, str> {
        user_action_naming_binary(self.kind, self.missing_binary)
    }
}

/// Maximum length (bytes) of any user-facing sanitized detail string.
pub const DETAIL_MAX_LEN: usize = 200;

/// The words whose presence in a token's KEY half means the value half is a
/// credential. Containment, not equality: `--client_secret=zzz` and
/// `--password=x` both have to match, and the suite pins the first of those.
const CREDENTIAL_KEY_WORDS: [&str; 8] = [
    "access_token",
    "refresh_token",
    "id_token",
    "api_key",
    "apikey",
    "password",
    "secret",
    "authorization",
];

/// True when a whitespace-separated token looks like a credential VALUE, or
/// like a `key=value` pair whose key names one.
///
/// The distinction is the whole point. A token that merely mentions
/// a credential word names something; it does not carry it. Before plugin confirmations
/// every string on this path was a literal this repo wrote, so a substring
/// test over the word was safe by inspection. A plugin's label is written by
/// the operator and templated against a row name, so `Describe db-password`
/// — an ordinary label over a real Secret Manager row — used to reach them as
/// `Describe [redacted]`.
fn looks_like_credential(token: &str) -> bool {
    let lower = token.to_lowercase();

    // Value shapes: these answer "is this token itself a credential", which is
    // the question the whole predicate should be asking. Unchanged.
    if lower.starts_with("bearer")
        || lower.contains("ya29.")
        || lower.starts_with("1//")
        || token.starts_with("AIza")
        || token.starts_with("eyJ")
    {
        return true;
    }

    // Asked of EVERY occurrence, not of the token's overall shape. An earlier
    // draft split the token at its first `=` (falling back to its first `:`)
    // and tested the key half; it leaked whenever the credential was not the
    // first pair — `https://host?a=1&password=hunter2` keys on
    // `https://host?a`, and `{"foo":"bar","password":"hunter2"}` keys on
    // `{"foo"`. The substring rule this replaces caught both, so that draft
    // traded over-redaction for under-redaction. The optional quotes are what
    // make the JSON shape work.
    //
    // Matching runs against a hyphen-normalised copy: gcloud spells its own
    // flags with hyphens, so `--api-key=zzz` is the likelier stderr shape than
    // `--api_key=zzz`, and an underscore-only word list missed every
    // multi-word key in its hyphenated spelling. The substitution is ASCII and
    // length-preserving, so byte indices stay on char boundaries.
    let normalized = lower.replace('-', "_");
    CREDENTIAL_KEY_WORDS.iter().any(|word| {
        normalized.match_indices(word).any(|(at, _)| {
            // The words are ASCII and the index came from `normalized`, so
            // slicing at the end of a match stays on a char boundary.
            let after = &normalized[at + word.len()..];
            // The word may only PREFIX the key: `secret_key=abc123`,
            // `--secret-key=abc`, `{"secret_key":"abc"}`. Consume the rest of
            // the key — the characters keys are actually made of — before
            // looking for the separator, or every such pair leaks its value.
            let after = after.trim_start_matches(|c: char| {
                c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')
            });
            let after = after.strip_prefix(['"', '\'']).unwrap_or(after);
            let Some(value) = after.strip_prefix('=').or_else(|| after.strip_prefix(':')) else {
                return false;
            };
            // An empty value hides nothing, so `password=` stays legible.
            !value.trim_start_matches(['"', '\'']).is_empty()
        })
    })
}

/// Redacts one whitespace-separated token, or strips its control characters.
///
/// Shared with [`crate::gcp::plugin_output`], which needs this rule and not
/// the flattening around it: a plugin's output is displayed as lines, so it
/// redacts token by token while keeping the lines those tokens sit on.
/// Extracting it is what keeps one credential rule rather than two — the credential
/// naming work narrowed this predicate once already, and a second copy would not have
/// been narrowed with it.
pub(crate) fn redact_token(token: &str) -> String {
    if looks_like_credential(token) {
        "[redacted]".to_string()
    } else {
        token.chars().filter(|c| !c.is_control()).collect()
    }
}

/// Redacts credential-shaped content, strips control characters, collapses
/// whitespace to single spaces, and bounds length. This is the single
/// chokepoint guaranteeing the UI never shows tokens or unbounded stderr.
pub fn sanitize_detail(text: &str) -> String {
    // Redact obvious secrets token-by-token.
    let redacted: String = text
        .split_whitespace()
        .map(redact_token)
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>()
        .join(" ");

    let trimmed = redacted.trim();
    if trimmed.len() <= DETAIL_MAX_LEN {
        trimmed.to_string()
    } else {
        // Reserve room for the trailing ellipsis so the *byte* length (what
        // callers and tests measure via `String::len`) never exceeds the bound.
        let ellipsis = '…';
        let budget = DETAIL_MAX_LEN.saturating_sub(ellipsis.len_utf8());
        let mut out = String::new();
        for c in trimmed.chars() {
            if out.len() + c.len_utf8() > budget {
                break;
            }
            out.push(c);
        }
        out.push(ellipsis);
        out
    }
}

/// A fully-formed, display-safe gcloud diagnostic: category, service, safe
/// command summary, exit code, and a sanitized bounded detail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GcloudDiagnostic {
    pub kind: GcloudErrorKind,
    pub service: String,
    pub command_summary: Option<String>,
    pub exit_code: Option<i32>,
    pub detail: String,
    /// Which binary was missing, for a failure that knows. `None` leaves the
    /// rendering vague ("missing CLI") rather than wrong, so a construction
    /// site that never sets it cannot resurrect the "install gcloud" advice
    /// for a missing `bq`.
    pub missing_binary: Option<&'static str>,
}

impl GcloudDiagnostic {
    pub fn new(
        kind: GcloudErrorKind,
        service: impl Into<String>,
        command_summary: Option<String>,
        exit_code: Option<i32>,
        raw_detail: &str,
    ) -> Self {
        Self {
            kind,
            service: service.into(),
            // Sanitized like the detail, and for the same reason. Every summary
            // used to be a static literal written in this repo, so passing it
            // through untouched was safe by inspection; a user-declared plugin's
            // label reaches it now, rendered against a row, so it can carry
            // control bytes, credential-shaped values and no length bound at
            // all. Doing it here rather than at the call sites is what keeps
            // `copy_text`'s "fully-sanitized" and `detail_preview`'s
            // display-safe claims true for every caller, including the ones not
            // written yet. Static literals are unaffected: they contain nothing
            // to redact and nothing to bound.
            command_summary: command_summary.as_deref().map(sanitize_detail),
            exit_code,
            detail: sanitize_detail(raw_detail),
            missing_binary: None,
        }
    }

    /// Records which binary was missing, so `label`/`user_action` can name it.
    ///
    /// A builder rather than a sixth parameter on [`new`](Self::new): only a
    /// `MissingExecutable` diagnostic has one, and `None` is not a silent
    /// wrong answer here — it renders the binary-agnostic wording.
    #[must_use]
    pub fn naming_missing_binary(mut self, binary: Option<&'static str>) -> Self {
        self.missing_binary = binary;
        self
    }

    /// The failure category in words, naming the missing binary when this
    /// diagnostic knows it — as opposed to [`GcloudErrorKind::label`], which
    /// cannot.
    pub fn label(&self) -> Cow<'static, str> {
        label_naming_binary(self.kind, self.missing_binary)
    }
    /// The remediation hint, naming the missing binary when known.
    pub fn user_action(&self) -> Cow<'static, str> {
        user_action_naming_binary(self.kind, self.missing_binary)
    }
    /// Whether this failure is worth retrying. A property of the kind alone —
    /// knowing which binary was missing changes the wording, never the answer.
    pub fn retryability(&self) -> Retryability {
        self.kind.retryability()
    }

    /// `"{label} — {user action}"` — the always-visible recovery hint.
    pub fn headline(&self) -> String {
        format!("{} — {}", self.label(), self.user_action())
    }

    /// Sanitized, already-bounded one-line technical detail (may be empty).
    pub fn detail_preview(&self) -> &str {
        &self.detail
    }

    /// Multi-line, fully-sanitized text for clipboard copy.
    pub fn copy_text(&self) -> String {
        let mut lines = vec![format!("{} [{}]", self.service, self.label())];
        if let Some(cmd) = &self.command_summary {
            lines.push(format!("command: {cmd}"));
        }
        if let Some(code) = self.exit_code {
            lines.push(format!("exit code {code}"));
        }
        lines.push(format!("action: {}", self.user_action()));
        if !self.detail.is_empty() {
            lines.push(format!("detail: {}", self.detail));
        }
        lines.join("\n")
    }
}

impl From<&GcloudDiagnostic> for DiagnosticProblem {
    fn from(d: &GcloudDiagnostic) -> Self {
        DiagnosticProblem::new(d.kind, d.detail.clone()).naming_missing_binary(d.missing_binary)
    }
}

/// Result of the targeted gcloud availability/auth probe.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GcloudDiagnosticsReport {
    /// Whether the gcloud binary ran at all (false = not installed / not spawnable).
    pub available: bool,
    /// Active account reported by `gcloud auth list`, when present.
    pub active_account: Option<String>,
    /// Classified problem when the probe failed or reported a problem.
    pub problem: Option<DiagnosticProblem>,
}
