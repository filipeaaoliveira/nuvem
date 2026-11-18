//! In-memory state for the Cloud Logging viewer: a bounded ring buffer of
//! `LogEntry` values with insert-id dedup, a page cursor for incremental
//! tailing, quota backoff (measured in poll ticks), a client-side filter, and
//! a status summarizing the buffer's health.
//!
//! Dedup/cap algorithm: `entries` is a `VecDeque` ordered oldest -> newest;
//! `seen` is a `HashSet<insert_id>` that mirrors it exactly. On append, an
//! entry is skipped if its `insert_id` is already in `seen`; otherwise it is
//! `push_back`-ed and its id `insert`-ed into `seen`. After appending, while
//! `entries.len() > LOG_BUFFER_CAP`, the oldest entry is `pop_front`-ed and
//! its id is `remove`-ed from `seen` -- so an evicted id is later treated as
//! unseen if it reappears. The cursor tracks the newest entry's timestamp
//! plus every insert id sharing that timestamp (GCP log timestamps are not
//! unique, so paging needs the full id set to avoid re-fetching the same
//! entries).

use std::collections::{HashSet, VecDeque};

use chrono::{DateTime, Local};

use nuvem_domain::gcp::diagnostics::GcloudErrorKind;
use nuvem_domain::gcp::logging::{
    LogCursor, LogEntry, LogQuery, LogSeverity, LogTimeRange, sanitize_log_text,
};
use nuvem_domain::gcp::text_filter::{TextFilter, TextFilterError};

pub const LOG_BUFFER_CAP: usize = 1000;
pub const LOG_PAGE_LIMIT: usize = 200;
pub const LOG_BACKOFF_MAX_TICKS: u32 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TailState {
    Running,
    Paused,
}

/// `Degraded` stores the bare `GcloudErrorKind`, not the failure carrier, so
/// the logging surfaces render `kind.label()` alone. A missing-binary failure
/// reads the binary-agnostic "missing CLI" here rather than naming `gcloud` or
/// `bq`: only a carrier records which binary was missing, and the Home health
/// card — whose probe does carry it — is what names it in the same frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoggingStatus {
    Initial,
    Loading,
    Loaded,
    HealthyEmpty,
    Degraded { kind: GcloudErrorKind },
    PartialParse { dropped: usize },
}

/// An applied search pattern together with the filter it compiled to.
///
/// The compile happens once per apply instead of once per call, which matters
/// because `visible_entries` runs on every render, every selection resync,
/// every copy and every export.
///
/// `regex::Regex` implements neither `PartialEq` nor `Eq`, so the compiled
/// filter cannot derive them — and does not need to. Every field here is a
/// pure function of `source`: `compiled` is produced by `TextFilter::parse` in
/// the constructor and is never mutated afterwards, and `source` is private,
/// so no caller can change one without the other. Two values with equal
/// sources are therefore equal in every observable respect, and equality
/// compares the source alone. That reasoning is the whole warrant for the
/// hand-written impl below: it holds only while the fields stay private and
/// the only way to build one is the constructor.
#[derive(Debug, Clone)]
pub struct CompiledTextFilter {
    source: String,
    compiled: Result<TextFilter, TextFilterError>,
}

impl PartialEq for CompiledTextFilter {
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source
    }
}

impl Eq for CompiledTextFilter {}

impl Default for CompiledTextFilter {
    fn default() -> Self {
        Self::new(String::new())
    }
}

impl CompiledTextFilter {
    fn new(source: String) -> Self {
        let compiled = TextFilter::parse(&source);
        Self { source, compiled }
    }

    /// The pattern the user applied, verbatim.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// The compiled filter, or `None` when the pattern does not parse.
    pub fn filter(&self) -> Option<&TextFilter> {
        self.compiled.as_ref().ok()
    }

    /// The rendered parse error, when there is one.
    fn error_text(&self) -> Option<String> {
        self.compiled.as_ref().err().map(|error| error.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoggingState {
    pub query: LogQuery,
    pub time_range: LogTimeRange,
    /// Minimum severity pushed into the gcloud filter, so the narrowing
    /// happens server-side. `LogSeverity::Default` is the sentinel for "no
    /// floor" and leaves the generated argv untouched.
    pub severity_floor: LogSeverity,
    pub tail: TailState,
    pub entries: VecDeque<LogEntry>,
    pub seen: HashSet<String>,
    pub cursor: Option<LogCursor>,
    pub selected: Option<usize>,
    pub selected_insert_id: Option<String>,
    pub detail_open: bool,
    pub wrap: bool,
    pub detail_scroll: u16,
    pub filter_draft: String,
    pub filter_active: bool,
    /// The applied search pattern and the filter it compiled to.
    ///
    /// Private on purpose: it is a cache, and the only thing that keeps it
    /// coherent is that no caller can write the source without going through
    /// [`LoggingState::apply_filter`], which recompiles. Read it with
    /// [`LoggingState::applied_filter`].
    applied_filter: CompiledTextFilter,
    pub status: LoggingStatus,
    pub backoff_ticks: u32,
    pub freshness: Option<DateTime<Local>>,
}

impl Default for LoggingState {
    fn default() -> Self {
        Self {
            query: LogQuery::ProjectLogs,
            time_range: LogTimeRange::DEFAULT,
            severity_floor: LogSeverity::Default,
            tail: TailState::Running,
            entries: VecDeque::new(),
            seen: HashSet::new(),
            cursor: None,
            selected: None,
            selected_insert_id: None,
            detail_open: false,
            wrap: false,
            detail_scroll: 0,
            filter_draft: String::new(),
            filter_active: false,
            applied_filter: CompiledTextFilter::default(),
            status: LoggingStatus::Initial,
            backoff_ticks: 0,
            freshness: None,
        }
    }
}

impl LoggingState {
    /// Replaces the buffer wholesale (e.g. after a fresh page load). Entries
    /// are deduped within the incoming page, capped to `LOG_BUFFER_CAP`, and
    /// the cursor/status/backoff/freshness are refreshed to reflect the new
    /// snapshot.
    pub fn apply_recent(&mut self, entries: Vec<LogEntry>, dropped: usize) {
        self.entries.clear();
        self.seen.clear();
        for entry in entries {
            self.push_unseen(entry);
        }
        self.cap_and_prune();
        self.cursor = Self::newest_cursor(&self.entries);
        self.backoff_ticks = 0;
        self.freshness = Some(Local::now());
        self.status = if dropped > 0 {
            LoggingStatus::PartialParse { dropped }
        } else if self.entries.is_empty() {
            LoggingStatus::HealthyEmpty
        } else {
            LoggingStatus::Loaded
        };
        self.resync_selection();
    }

    /// Appends only entries whose `insert_id` has not already been seen (as
    /// determined by tracking `seen`), then caps and prunes the buffer. Used
    /// for incremental tail polls.
    pub fn apply_tail(&mut self, entries: Vec<LogEntry>, dropped: usize) {
        let mut appended_any = false;
        for entry in entries {
            if self.push_unseen(entry) {
                appended_any = true;
            }
        }
        self.cap_and_prune();
        if appended_any {
            self.cursor = Self::newest_cursor(&self.entries);
        }
        self.backoff_ticks = 0;
        self.freshness = Some(Local::now());
        self.status = if dropped > 0 {
            LoggingStatus::PartialParse { dropped }
        } else if self.entries.is_empty() {
            LoggingStatus::HealthyEmpty
        } else {
            LoggingStatus::Loaded
        };
        self.resync_selection();
    }

    /// Records a gcloud failure. Existing entries are left in place (now
    /// considered stale) so the last-known view remains visible. A `Quota`
    /// error doubles `backoff_ticks` (starting from at least 1), capped at
    /// `LOG_BACKOFF_MAX_TICKS`; other error kinds leave backoff untouched.
    pub fn record_error(&mut self, kind: GcloudErrorKind) {
        self.status = LoggingStatus::Degraded { kind };
        if kind == GcloudErrorKind::Quota {
            self.backoff_ticks = (self.backoff_ticks.max(1) * 2).min(LOG_BACKOFF_MAX_TICKS);
        }
    }

    /// Decrements the backoff counter by one poll tick (saturating at zero)
    /// and returns whether it is now clear, i.e. a tail poll may proceed.
    pub fn decrement_backoff(&mut self) -> bool {
        if self.backoff_ticks > 0 {
            self.backoff_ticks -= 1;
        }
        self.backoff_ticks == 0
    }

    pub fn tail_is_running(&self) -> bool {
        matches!(self.tail, TailState::Running)
    }

    pub fn toggle_tail(&mut self) {
        self.tail = match self.tail {
            TailState::Running => TailState::Paused,
            TailState::Paused => TailState::Running,
        };
    }

    /// Advances to the next time range preset and clears the buffer/cursor,
    /// setting status to `Loading` since the new window requires a fresh
    /// fetch.
    pub fn cycle_time_range(&mut self) {
        self.time_range = self.time_range.next();
        self.clear_for_reload();
    }

    /// Jumps straight to `range`. Returns whether the window actually moved;
    /// re-selecting the active bucket is a no-op that keeps the buffer, so a
    /// repeated key press never wipes a populated view for an identical
    /// refetch.
    pub fn set_time_range(&mut self, range: LogTimeRange) -> bool {
        if self.time_range == range {
            return false;
        }
        self.time_range = range;
        self.clear_for_reload();
        true
    }

    /// Steps the server-side severity floor one rank up. Returns whether the
    /// floor actually moved: at `Emergency` the step is clamped, and a clamped
    /// step must not clear the buffer (the fetch it would trigger returns the
    /// same entries).
    pub fn raise_severity_floor(&mut self) -> bool {
        self.set_severity_floor(self.severity_floor.raise())
    }

    /// Steps the severity floor one rank down, clamped at the
    /// `LogSeverity::Default` no-floor sentinel. Returns whether it moved; see
    /// [`LoggingState::raise_severity_floor`].
    pub fn lower_severity_floor(&mut self) -> bool {
        self.set_severity_floor(self.severity_floor.lower())
    }

    fn set_severity_floor(&mut self, floor: LogSeverity) -> bool {
        if self.severity_floor == floor {
            return false;
        }
        self.severity_floor = floor;
        self.clear_for_reload();
        true
    }

    /// Drops everything derived from the previous query window -- entries,
    /// dedup set, cursor, selection -- and marks the buffer as awaiting a
    /// fresh fetch. Callers are responsible for emitting that fetch.
    fn clear_for_reload(&mut self) {
        self.drop_buffer();
        self.status = LoggingStatus::Loading;
    }

    /// Returns the viewer to "nothing loaded, nothing in flight" -- for a
    /// caller that meant to reload and found there was nothing to fetch.
    ///
    /// The buffer goes with the status. `Initial` renders as "Not loaded", so
    /// keeping rows under it would put a header that denies a load directly
    /// above the rows a load produced; and those rows were fetched for a
    /// context that no longer resolves, so nothing will refresh or extend them.
    ///
    /// The detail pane goes with the buffer, and closing it belongs here rather
    /// than in `LoggingState::drop_buffer`: the other clearing path is only
    /// ever reached from list-mode keys, which cannot run with the pane up, so
    /// moving it down would change nothing there but its documented meaning.
    /// This path *can* arrive with the pane open -- a restore into Logging runs
    /// whatever the mode was when the workspace changed -- and the pane renders
    /// off the selection this drops, so leaving `detail_open` set would keep a
    /// keyboard mode alive with nothing on screen behind it.
    pub fn settle_unloaded(&mut self) {
        self.drop_buffer();
        self.detail_open = false;
        self.detail_scroll = 0;
        self.status = LoggingStatus::Initial;
    }

    /// Settles a spinner that nothing will ever clear, for a caller that meant
    /// to reload, found there was nothing to fetch, and has rows worth keeping.
    ///
    /// Only `Loading` moves. That status is the one claim a skipped fetch makes
    /// false -- it says a page is in flight when none was dispatched, and with
    /// no context the tail poll skips too, so nothing else would ever take it
    /// down. Every other status already reports a real last-known outcome:
    /// `HealthyEmpty` means a fetch ran and the window held nothing, `Initial`
    /// means nothing was ever fetched. A refetch that never left learned nothing
    /// that contradicts either, so overwriting them would replace a true
    /// statement with a vaguer one.
    ///
    /// Where `Loading` lands is decided by what is rendered underneath it, since
    /// that is the whole point of settling: `Loaded` over rows, `Initial` over an
    /// empty buffer. The buffer, selection, cursor and detail pane are left
    /// alone -- those rows were really fetched, and an operator may be mid-read.
    pub fn settle_keeping_buffer(&mut self) {
        if matches!(self.status, LoggingStatus::Loading) {
            self.status = if self.entries.is_empty() {
                LoggingStatus::Initial
            } else {
                LoggingStatus::Loaded
            };
        }
    }

    /// Records that a first page is now in flight, for the one caller that
    /// dispatches such a fetch without having cleared the buffer for it: the
    /// background tail poll, which loads the first page when there is no cursor
    /// yet.
    ///
    /// Only `Initial` moves. That status means "nothing was ever fetched", and
    /// both the header and the body say exactly that, so leaving it up while a
    /// fetch is in flight would deny a load that is running. Every other status
    /// already reports a real last-known outcome, and overwriting it per poll
    /// tick would flicker a loaded-but-empty view between its result and a
    /// spinner.
    pub fn begin_first_load(&mut self) {
        if matches!(self.status, LoggingStatus::Initial) {
            self.status = LoggingStatus::Loading;
        }
    }

    /// Clears the buffer and everything derived from it, leaving the status to
    /// the caller -- it is the one thing the two clearing paths disagree on.
    fn drop_buffer(&mut self) {
        self.entries.clear();
        self.seen.clear();
        self.cursor = None;
        self.selected = None;
        self.selected_insert_id = None;
    }

    pub fn toggle_wrap(&mut self) {
        self.wrap = !self.wrap;
    }

    pub fn select_next(&mut self) {
        let len = self.visible_entries().len();
        if len == 0 {
            self.selected = None;
            self.selected_insert_id = None;
            return;
        }
        let new_index = match self.selected {
            Some(index) if index + 1 < len => index + 1,
            Some(index) => index,
            None => 0,
        };
        self.selected = Some(new_index);
        self.selected_insert_id = self
            .visible_entries()
            .get(new_index)
            .map(|entry| entry.insert_id.clone());
    }

    pub fn select_previous(&mut self) {
        let len = self.visible_entries().len();
        if len == 0 {
            self.selected = None;
            self.selected_insert_id = None;
            return;
        }
        let new_index = match self.selected {
            Some(index) if index > 0 => index - 1,
            Some(_) => 0,
            None => 0,
        };
        self.selected = Some(new_index);
        self.selected_insert_id = self
            .visible_entries()
            .get(new_index)
            .map(|entry| entry.insert_id.clone());
    }

    pub fn open_detail(&mut self) {
        if self.selected.is_some() {
            self.detail_open = true;
            self.detail_scroll = 0;
        }
    }

    pub fn close_detail(&mut self) {
        self.detail_open = false;
        self.detail_scroll = 0;
    }

    pub fn scroll_detail(&mut self, delta: i16) {
        let current = i32::from(self.detail_scroll);
        let next = (current + i32::from(delta)).max(0);
        self.detail_scroll = u16::try_from(next).unwrap_or(u16::MAX);
    }

    /// The applied search pattern. The compiled form is a private cache; this
    /// is the only way to read the source it was compiled from.
    pub fn applied_filter(&self) -> &str {
        self.applied_filter.source()
    }

    /// The rendered parse error of the applied pattern, when it does not
    /// compile. Asked of the cache on every read rather than copied into a
    /// field beside it: a second copy is a second thing that can disagree with
    /// the filter the rows were actually matched against, and it would be
    /// writable by callers the cache itself keeps out.
    pub fn applied_filter_error(&self) -> Option<String> {
        self.applied_filter.error_text()
    }

    pub fn start_filter(&mut self) {
        self.filter_active = true;
        self.filter_draft = self.applied_filter.source().to_string();
    }

    pub fn append_filter_char(&mut self, ch: char) {
        self.filter_draft.push(ch);
    }

    pub fn backspace_filter(&mut self) {
        self.filter_draft.pop();
    }

    /// Promotes the draft to the applied filter. This is the only writer of
    /// the applied pattern, and it recompiles in the same breath — which is
    /// what makes caching the compiled filter safe.
    pub fn apply_filter(&mut self) {
        self.applied_filter = CompiledTextFilter::new(self.filter_draft.clone());
        self.filter_active = false;
        self.resync_selection();
    }

    pub fn cancel_filter(&mut self) {
        self.filter_draft = self.applied_filter.source().to_string();
        self.filter_active = false;
    }

    /// Entries matching the applied filter under the log [`TextFilter`]
    /// grammar (substring by default, `~` fuzzy, `/…/` regex, `!` inverts),
    /// searched over the message, severity label, and resource type. Returns
    /// all entries when the pattern is empty, and *no* entries when it fails
    /// to parse -- showing everything would silently contradict the filter the
    /// user typed, so the empty view plus [`LoggingState::applied_filter_error`]
    /// is what tells
    /// them to fix it.
    ///
    /// The filter is compiled once per apply and read from the cache here.
    /// This runs on every render, selection resync, copy and export, so it
    /// must not compile a regex.
    pub fn visible_entries(&self) -> Vec<&LogEntry> {
        let Some(filter) = self.applied_filter.filter() else {
            return Vec::new();
        };
        if filter.is_match_all() {
            return self.entries.iter().collect();
        }
        self.entries
            .iter()
            .filter(|entry| {
                filter.matches_any([
                    entry.message.as_str(),
                    entry.severity.label(),
                    entry.resource_type.as_str(),
                ])
            })
            .collect()
    }

    /// The visible entries rendered one per line as
    /// `<timestamp> <SEVERITY> <resource_type> <message>`, oldest first. Every
    /// log-derived field is passed through [`sanitize_log_text`] so a dump can
    /// never carry terminal control sequences into whatever opens the file
    /// next. The severity label is a fixed enum rendering, so it needs no
    /// sanitizing.
    pub fn visible_export_lines(&self) -> Vec<String> {
        self.visible_entries()
            .into_iter()
            .map(|entry| {
                format!(
                    "{} {} {} {}",
                    sanitize_log_text(&entry.timestamp),
                    entry.severity.label(),
                    sanitize_log_text(&entry.resource_type),
                    sanitize_log_text(&entry.message),
                )
            })
            .collect()
    }

    /// Switches to a new query target: clears the buffer/cursor/dedup set,
    /// resumes tailing, and marks status as `Loading` pending the first
    /// fetch.
    pub fn open_query(&mut self, query: LogQuery) {
        self.query = query;
        self.clear_for_reload();
        self.tail = TailState::Running;
    }

    /// The cursor for the newest entry in `entries`: its timestamp plus every
    /// insert id sharing that timestamp. `None` when `entries` is empty.
    pub fn newest_cursor(entries: &VecDeque<LogEntry>) -> Option<LogCursor> {
        let newest_timestamp = &entries.back()?.timestamp;
        let insert_ids: Vec<String> = entries
            .iter()
            .filter(|entry| &entry.timestamp == newest_timestamp)
            .map(|entry| entry.insert_id.clone())
            .collect();
        Some(LogCursor {
            timestamp: newest_timestamp.clone(),
            insert_ids,
        })
    }

    /// Pushes `entry` if unseen, tracking it in `seen`. Returns whether it
    /// was appended.
    fn push_unseen(&mut self, entry: LogEntry) -> bool {
        if self.seen.contains(&entry.insert_id) {
            return false;
        }
        self.seen.insert(entry.insert_id.clone());
        self.entries.push_back(entry);
        true
    }

    /// Evicts oldest entries until the buffer is within `LOG_BUFFER_CAP`,
    /// pruning `seen` in lockstep so evicted ids are treated as unseen again.
    fn cap_and_prune(&mut self) {
        while self.entries.len() > LOG_BUFFER_CAP {
            if let Some(evicted) = self.entries.pop_front() {
                self.seen.remove(&evicted.insert_id);
            }
        }
    }

    /// Re-anchors `selected` after a buffer mutation (append, front-eviction, or
    /// filter change), and seeds it when there is nothing to re-anchor.
    ///
    /// Anchoring is by `insert_id` rather than by index, so the entry under the
    /// cursor stays under it as the tail pushes new entries in above. That is
    /// what makes the seeding below safe: an automatic selection cannot slide
    /// onto whatever is newest and take the operator's place from them.
    ///
    /// With no anchor at all — a fresh list, or one whose anchor a previous
    /// update cleared — the cursor goes to the first visible entry.
    /// Without that, `selected` stayed `None` until `j` was pressed, and
    /// `Enter` was inert while the footer advertised `Enter Open entry` from the
    /// first frame, with no cursor on screen to say why. `y` and the
    /// copy-insert-id key were inert for the same reason.
    ///
    /// Seeding deliberately does not apply in the update that *loses* an anchor;
    /// see the comment on that arm.
    ///
    /// An empty list gets no cursor: there is nothing to point at, and a cursor
    /// on no row is a claim the list cannot keep.
    fn resync_selection(&mut self) {
        if let Some(id) = &self.selected_insert_id {
            let position = self
                .visible_entries()
                .iter()
                .position(|entry| &entry.insert_id == id);
            match position {
                Some(index) => self.selected = Some(index),
                // The anchored entry is gone: clear rather than re-seed *in
                // this update*. That decision predates the seeding below and is
                // its own — `selection_clears_when_selected_entry_is_evicted`
                // states it — and the reason still holds: the operator's
                // selection vanished because its subject did, and moving the
                // cursor onto a different entry in the same breath is the drift
                // that decision refuses. The next update sees no anchor and
                // seeds like a fresh list, so the cursor comes back on the poll
                // after rather than never.
                None => {
                    self.selected = None;
                    self.selected_insert_id = None;
                }
            }
            return;
        }

        match self.visible_entries().first() {
            Some(entry) => {
                let id = entry.insert_id.clone();
                self.selected = Some(0);
                self.selected_insert_id = Some(id);
            }
            None => {
                self.selected = None;
                self.selected_insert_id = None;
            }
        }
    }
}
