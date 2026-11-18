//! Pub/Sub topics and subscriptions in application state.
//!
//! # Two tables over one list call
//!
//! A subscription is a first-class, listable resource, and
//! `gcloud pubsub subscriptions list` returns every subscription in the
//! project with the parent `topic` on each row. That single fact is what lets
//! both tables here exist off one call: [`PubSubSubscriptions`] renders those
//! rows directly, and [`PubSubTopics`] groups the same rows by topic to derive
//! its count column — client-side, from **one** call rather than one per
//! topic.
//!
//! This is the difference from Cloud Run revisions, which are reachable only
//! per-service and so can only ever be a drill-down. Subscriptions are not
//! nested under topics here; they are a sibling table, and a subscription
//! whose topic has been deleted (`_deleted-topic_`) is a row the flat table
//! shows and the count grouping skips.
//!
//! The two tables share the call and nothing else: the row type, the sort
//! fields, the searchable set and the failure policy are each their own. See
//! `crate::services::resources::pubsub` for why one call hands out a `Result`
//! and lets each caller decide what a refusal means.
//!
//! # List-only, both of them
//!
//! Neither table has a detail pane: `Detail` and `DetailTab` are `()` on both
//! services, so `Enter` on a row does nothing at all. That is the shape Cloud
//! Functions shipped in and stayed in between its own PRs, and it is
//! deliberate rather than unfinished — a pane that opens on nothing is worse
//! than one not offered yet, which is also why there is no detail tab enum
//! here carrying variants nothing renders.
//!
//! In particular there is **no topic-detail Subscriptions tab**, and the flat
//! table is not a replacement for one that was never built. Every field a pane
//! would want already arrives, since both tables parse the same rows in full;
//! what a later slice would add is the presentation.
//!
//! # The count degrades on its own
//!
//! `pubsub.topics.list` granted with `pubsub.subscriptions.list` denied is an
//! ordinary read-only IAM shape. When the subscriptions call fails, the topics
//! table still renders on its other four columns and the count reads as
//! unknown — see [`SubscriptionCount`]. A topic list that fails wholesale
//! because a secondary enrichment call failed is worse than one with no count.

use super::*;

/// Marker type unifying the Pub/Sub topics service behind [`ResourceService`].
/// Never instantiated.
pub enum PubSubTopics {}

impl ResourceService for PubSubTopics {
    type Row = PubSubTopic;
    type Detail = ();
    type SortField = PubSubTopicSortField;
    type DetailTab = ();

    const KIND: ResourceServiceKind = ResourceServiceKind::PubSubTopics;

    fn collection(
        state: &ResourcesState,
    ) -> &ResourceCollectionState<Self::Row, Self::Detail, Self::SortField, Self::DetailTab> {
        &state.pubsub_topics
    }

    fn collection_mut(
        state: &mut ResourcesState,
    ) -> &mut ResourceCollectionState<Self::Row, Self::Detail, Self::SortField, Self::DetailTab>
    {
        &mut state.pubsub_topics
    }

    fn matches_filter(row: &Self::Row, filter: &CompiledFilter) -> bool {
        topic_matches_filter(row, filter)
    }

    /// The needle sits in the short name, the identity column of the topic
    /// table, and in the full resource name it is derived from.
    fn sample_row() -> Self::Row {
        PubSubTopic {
            name: SAMPLE_ROW_NEEDLE.to_string(),
            full_name: format!("projects/sample-project/topics/{SAMPLE_ROW_NEEDLE}"),
            subscription_count: SubscriptionCount::Known(1),
            message_retention: None,
            kms_key: None,
            labels: Vec::new(),
        }
    }

    fn compare(
        left: &Self::Row,
        right: &Self::Row,
        sort_field: Self::SortField,
        direction: SortDirection,
    ) -> std::cmp::Ordering {
        compare_topics(left, right, sort_field, direction)
    }

    /// A topic is global and its id is unique within the project, so the name
    /// alone is the identity and there is no location to qualify it with —
    /// the Secret Manager shape, not the Cloud Functions one.
    fn row_id(row: &Self::Row) -> ResourceRowId {
        ResourceRowId {
            name: row.name.clone(),
            location: String::new(),
        }
    }
}

/// The sortable columns of the topics table.
///
/// One variant per *sortable* catalogue column. The catalogue has five columns
/// and four of them sort: `labels` does not, for the reason recorded beside
/// `PUBSUB_TOPIC_COLUMNS`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PubSubTopicSortField {
    Name,
    Subscriptions,
    Retention,
    KmsKey,
}

impl PubSubTopicSortField {
    pub fn label(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::Subscriptions => "Subscriptions",
            Self::Retention => "Retention",
            Self::KmsKey => "KMS key",
        }
    }

    pub const COLUMN_COUNT: usize = 5;

    pub fn from_column(index: usize) -> Option<Self> {
        match index {
            0 => Some(Self::Name),
            1 => Some(Self::Subscriptions),
            2 => Some(Self::Retention),
            3 => Some(Self::KmsKey),
            // 4 is `labels`, the one catalogue column with no sort field.
            _ => None,
        }
    }

    pub fn column(self) -> usize {
        match self {
            Self::Name => 0,
            Self::Subscriptions => 1,
            Self::Retention => 2,
            Self::KmsKey => 3,
        }
    }
}

/// Marker type unifying the Pub/Sub subscriptions service behind
/// [`ResourceService`]. Never instantiated.
pub enum PubSubSubscriptions {}

impl ResourceService for PubSubSubscriptions {
    type Row = PubSubSubscription;
    type Detail = ();
    type SortField = PubSubSubscriptionSortField;
    type DetailTab = ();

    const KIND: ResourceServiceKind = ResourceServiceKind::PubSubSubscriptions;

    fn collection(
        state: &ResourcesState,
    ) -> &ResourceCollectionState<Self::Row, Self::Detail, Self::SortField, Self::DetailTab> {
        &state.pubsub_subscriptions
    }

    fn collection_mut(
        state: &mut ResourcesState,
    ) -> &mut ResourceCollectionState<Self::Row, Self::Detail, Self::SortField, Self::DetailTab>
    {
        &mut state.pubsub_subscriptions
    }

    fn matches_filter(row: &Self::Row, filter: &CompiledFilter) -> bool {
        subscription_matches_filter(row, filter)
    }

    /// The needle sits in the short name, the identity column of the
    /// subscription table. The parent topic deliberately does not carry it:
    /// a subscription found only through its topic's name would make this
    /// walk pass on a `matches_filter` reading the wrong row entirely.
    fn sample_row() -> Self::Row {
        PubSubSubscription {
            name: SAMPLE_ROW_NEEDLE.to_string(),
            full_name: format!("projects/sample-project/subscriptions/{SAMPLE_ROW_NEEDLE}"),
            topic: "orders".to_string(),
            delivery: PubSubDelivery::Pull,
            ack_deadline_seconds: Some(10),
            message_retention: None,
            dead_letter: None,
            state: PubSubSubscriptionState::Active,
            labels: Vec::new(),
        }
    }

    fn compare(
        left: &Self::Row,
        right: &Self::Row,
        sort_field: Self::SortField,
        direction: SortDirection,
    ) -> std::cmp::Ordering {
        compare_subscriptions(left, right, sort_field, direction)
    }

    /// Like a topic, a subscription is global and its id is unique within the
    /// project, so the name alone is the identity with no location to qualify
    /// it. The parent topic is deliberately *not* part of the identity: two
    /// subscriptions on different topics cannot share a name, and folding the
    /// topic in would break the cursor the moment a subscription outlived its
    /// topic and started reading `_deleted-topic_`.
    fn row_id(row: &Self::Row) -> ResourceRowId {
        ResourceRowId {
            name: row.name.clone(),
            location: String::new(),
        }
    }
}

/// The sortable columns of the subscriptions table.
///
/// One variant per *sortable* catalogue column. The catalogue has eight
/// columns and seven of them sort: `labels` does not, for the same reason it
/// does not on the topics table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PubSubSubscriptionSortField {
    Name,
    Topic,
    Delivery,
    State,
    AckDeadline,
    Retention,
    DeadLetter,
}

impl PubSubSubscriptionSortField {
    pub fn label(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::Topic => "Topic",
            Self::Delivery => "Delivery",
            Self::State => "State",
            Self::AckDeadline => "Ack deadline",
            Self::Retention => "Retention",
            Self::DeadLetter => "Dead letter",
        }
    }

    pub const COLUMN_COUNT: usize = 8;

    pub fn from_column(index: usize) -> Option<Self> {
        match index {
            0 => Some(Self::Name),
            1 => Some(Self::Topic),
            2 => Some(Self::Delivery),
            3 => Some(Self::State),
            4 => Some(Self::AckDeadline),
            5 => Some(Self::Retention),
            6 => Some(Self::DeadLetter),
            // 7 is `labels`, the one catalogue column with no sort field.
            _ => None,
        }
    }

    pub fn column(self) -> usize {
        match self {
            Self::Name => 0,
            Self::Topic => 1,
            Self::Delivery => 2,
            Self::State => 3,
            Self::AckDeadline => 4,
            Self::Retention => 5,
            Self::DeadLetter => 6,
        }
    }
}

impl ResourcesState {
    pub fn pubsub_topics(&self) -> &[PubSubTopic] {
        &self.pubsub_topics.rows
    }

    pub fn visible_pubsub_topics(&self) -> Vec<&PubSubTopic> {
        let filter = self.pubsub_topics.filter_applied.filter();
        let mut rows = self
            .pubsub_topics
            .rows
            .iter()
            .filter(|topic| topic_matches_filter(topic, filter))
            .collect::<Vec<_>>();
        rows.sort_by(|left, right| {
            compare_topics(
                left,
                right,
                self.pubsub_topics.sort_field,
                self.pubsub_topics.sort_direction,
            )
        });
        rows
    }

    pub fn visible_pubsub_topic_count(&self) -> usize {
        let filter = self.pubsub_topics.filter_applied.filter();
        self.pubsub_topics
            .rows
            .iter()
            .filter(|topic| topic_matches_filter(topic, filter))
            .count()
    }

    pub fn selected_pubsub_topic(&self) -> Option<&PubSubTopic> {
        self.visible_pubsub_topics()
            .get(self.pubsub_topics.selected_visible)
            .copied()
    }

    /// Installs a freshly loaded list, keeping the cursor on the topic it was
    /// already on where that topic is still listed.
    pub fn set_pubsub_topics(&mut self, context: GcloudContext, topics: Vec<PubSubTopic>) {
        let previously_selected = self.selected_pubsub_topic().map(|topic| topic.name.clone());

        self.pubsub_topics.context = Some(context);
        self.pubsub_topics.rows = topics;
        self.pubsub_topics.stale = false;
        self.pubsub_topics.last_problem = None;
        self.pubsub_topics.last_diagnostic = None;
        self.pubsub_topics.clear_load_phase();

        self.pubsub_topics.selected_visible = previously_selected
            .as_ref()
            .and_then(|name| {
                self.visible_pubsub_topics()
                    .iter()
                    .position(|topic| &topic.name == name)
            })
            .unwrap_or_else(|| {
                self.pubsub_topics
                    .selected_visible
                    .min(self.visible_pubsub_topics().len().saturating_sub(1))
            });
    }

    pub fn mark_pubsub_topics_stale_and_clear(&mut self) {
        self.pubsub_topics.filter_draft = self.pubsub_topics.filter_applied.source().to_string();
        self.pubsub_topics.filter_input_active = false;
        self.pubsub_topics.rows.clear();
        self.pubsub_topics.context = None;
        self.pubsub_topics.selected_visible = 0;
        self.pubsub_topics.stale = true;
    }

    pub fn pubsub_topics_problem(&self) -> Option<&DiagnosticProblem> {
        self.pubsub_topics.last_problem.as_ref()
    }

    pub fn pubsub_subscriptions(&self) -> &[PubSubSubscription] {
        &self.pubsub_subscriptions.rows
    }

    pub fn visible_pubsub_subscriptions(&self) -> Vec<&PubSubSubscription> {
        let filter = self.pubsub_subscriptions.filter_applied.filter();
        let mut rows = self
            .pubsub_subscriptions
            .rows
            .iter()
            .filter(|subscription| subscription_matches_filter(subscription, filter))
            .collect::<Vec<_>>();
        rows.sort_by(|left, right| {
            compare_subscriptions(
                left,
                right,
                self.pubsub_subscriptions.sort_field,
                self.pubsub_subscriptions.sort_direction,
            )
        });
        rows
    }

    pub fn visible_pubsub_subscription_count(&self) -> usize {
        let filter = self.pubsub_subscriptions.filter_applied.filter();
        self.pubsub_subscriptions
            .rows
            .iter()
            .filter(|subscription| subscription_matches_filter(subscription, filter))
            .count()
    }

    pub fn selected_pubsub_subscription(&self) -> Option<&PubSubSubscription> {
        self.visible_pubsub_subscriptions()
            .get(self.pubsub_subscriptions.selected_visible)
            .copied()
    }

    /// Installs a freshly loaded list, keeping the cursor on the subscription
    /// it was already on where that subscription is still listed.
    pub fn set_pubsub_subscriptions(
        &mut self,
        context: GcloudContext,
        subscriptions: Vec<PubSubSubscription>,
    ) {
        let previously_selected = self
            .selected_pubsub_subscription()
            .map(|subscription| subscription.name.clone());

        self.pubsub_subscriptions.context = Some(context);
        self.pubsub_subscriptions.rows = subscriptions;
        self.pubsub_subscriptions.stale = false;
        self.pubsub_subscriptions.last_problem = None;
        self.pubsub_subscriptions.last_diagnostic = None;
        self.pubsub_subscriptions.clear_load_phase();

        self.pubsub_subscriptions.selected_visible = previously_selected
            .as_ref()
            .and_then(|name| {
                self.visible_pubsub_subscriptions()
                    .iter()
                    .position(|subscription| &subscription.name == name)
            })
            .unwrap_or_else(|| {
                self.pubsub_subscriptions
                    .selected_visible
                    .min(self.visible_pubsub_subscriptions().len().saturating_sub(1))
            });
    }

    pub fn mark_pubsub_subscriptions_stale_and_clear(&mut self) {
        self.pubsub_subscriptions.filter_draft = self
            .pubsub_subscriptions
            .filter_applied
            .source()
            .to_string();
        self.pubsub_subscriptions.filter_input_active = false;
        self.pubsub_subscriptions.rows.clear();
        self.pubsub_subscriptions.context = None;
        self.pubsub_subscriptions.selected_visible = 0;
        self.pubsub_subscriptions.stale = true;
    }

    pub fn pubsub_subscriptions_problem(&self) -> Option<&DiagnosticProblem> {
        self.pubsub_subscriptions.last_problem.as_ref()
    }
}

/// The values a topics filter searches.
///
/// Every cell the table paints, because `/` narrows what is on screen and a
/// term an operator can read in a column but cannot filter by is the filter
/// lying about the rows it hid.
///
/// The subscription count contributes its *display label*, which is the digits
/// the cell carries — and, for a count that could not be fetched, the same
/// dash the cell shows rather than a `0` no column ever painted.
///
/// The never-drop-a-row premise holds trivially here because this service
/// pushes no regex clause down at all (`FILTER_FIELDS` in
/// `crate::services::resources::pubsub` is empty), so every one of these
/// values is matched client-side against a row that was never narrowed
/// server-side.
fn topic_searchable_fields(topic: &PubSubTopic) -> Vec<String> {
    let mut fields = vec![
        topic.name.clone(),
        topic.subscription_count.label(),
        topic.message_retention.clone().unwrap_or_default(),
        topic.kms_key.clone().unwrap_or_default(),
    ];
    fields.extend(
        topic
            .labels
            .iter()
            .map(|label| format!("{}={}", label.key, label.value)),
    );
    fields
}

fn topic_matches_filter(topic: &PubSubTopic, filter: &CompiledFilter) -> bool {
    filter.matches(&topic_searchable_fields(topic))
}

fn compare_topics(
    left: &PubSubTopic,
    right: &PubSubTopic,
    sort_field: PubSubTopicSortField,
    direction: SortDirection,
) -> std::cmp::Ordering {
    let primary = match sort_field {
        PubSubTopicSortField::Name => left.name.cmp(&right.name),
        // Numerically, not by the rendered label: `10` sorts after `9` in the
        // column an operator sorted to find the busiest topic, which text
        // ordering gets backwards. A count that could not be fetched sorts as
        // if it were absent rather than as zero, so the unknowns group at one
        // end instead of claiming to be the quietest topics.
        PubSubTopicSortField::Subscriptions => left
            .subscription_count
            .known()
            .cmp(&right.subscription_count.known()),
        // RFC 3339 durations as the API returns them (`604800s`); an unset
        // retention is the empty string, so those rows group together rather
        // than scattering through the ordering.
        PubSubTopicSortField::Retention => left
            .message_retention
            .as_deref()
            .unwrap_or_default()
            .cmp(right.message_retention.as_deref().unwrap_or_default()),
        PubSubTopicSortField::KmsKey => left
            .kms_key
            .as_deref()
            .unwrap_or_default()
            .cmp(right.kms_key.as_deref().unwrap_or_default()),
    };
    // The name breaks every tie and is unique within the project, so this is
    // a total order over the row set.
    let ordering = primary.then_with(|| left.name.cmp(&right.name));
    match direction {
        SortDirection::Ascending => ordering,
        SortDirection::Descending => ordering.reverse(),
    }
}

/// The values a subscriptions filter searches.
///
/// Every cell the table paints, for the same reason the topics table does it:
/// a term an operator can read in a column but cannot filter by is the filter
/// lying about the rows it hid.
///
/// Two of these are *rendered* values with no raw counterpart — the delivery
/// label (`Pull`/`Push`) and the state label (`Resource error` against a raw
/// `RESOURCE_ERROR`). That is exactly why this service pushes no regex clause
/// down at all (`SUBSCRIPTION_FILTER_FIELDS` in
/// `crate::services::resources::pubsub` is empty), so the never-drop-a-row
/// premise holds trivially: every value here is matched client-side against a
/// row that was never narrowed server-side.
///
/// The parent topic contributes its short id, which is what the `TOPIC` column
/// paints. A subscription whose topic is gone contributes the sentinel
/// `_deleted-topic_` verbatim, because that is what its cell reads and an
/// operator hunting orphans will type it.
fn subscription_searchable_fields(subscription: &PubSubSubscription) -> Vec<String> {
    let mut fields = vec![
        subscription.name.clone(),
        parent_topic_label(subscription),
        subscription.delivery.label().to_string(),
        subscription.state.label().to_string(),
        subscription
            .ack_deadline_seconds
            .map(|seconds| seconds.to_string())
            .unwrap_or_default(),
        subscription.message_retention.clone().unwrap_or_default(),
        dead_letter_label(subscription).unwrap_or_default(),
    ];
    // A push endpoint is searchable but has no column of its own: it is the
    // one fact an operator hunting a broken push subscription has to go on,
    // and `DELIVERY` renders only `Push`. Searchable-without-a-column is the
    // safe direction of the never-drop-a-row invariant — it can only widen
    // what matches, never hide a row that is on screen.
    if let Some(endpoint) = subscription.delivery.endpoint() {
        fields.push(endpoint.to_string());
    }
    fields.extend(
        subscription
            .labels
            .iter()
            .map(|label| format!("{}={}", label.key, label.value)),
    );
    fields
}

fn subscription_matches_filter(subscription: &PubSubSubscription, filter: &CompiledFilter) -> bool {
    filter.matches(&subscription_searchable_fields(subscription))
}

/// What the `TOPIC` column paints: the topic's short id, or the API's
/// deleted-topic sentinel carried verbatim.
///
/// The sentinel is not a slash-separated resource path, so shortening it would
/// be a no-op anyway — but it is spelled out here because it is a real value an
/// operator reads and filters on, not an error state to blank out.
pub fn parent_topic_label(subscription: &PubSubSubscription) -> String {
    short_id(&subscription.topic)
}

/// What the `DEAD LETTER` column paints: the dead-letter topic's short id, or
/// `None` where no policy is configured.
///
/// Beside [`parent_topic_label`] rather than in the projection, for the reason
/// that one is here: these are the two cells on this table that render a
/// resource path, and the projection had its own copy of the shortening until
/// the second one arrived and made the duplication a drift risk rather than a
/// coincidence.
pub fn dead_letter_label(subscription: &PubSubSubscription) -> Option<String> {
    subscription
        .dead_letter
        .as_ref()
        .map(|policy| short_id(&policy.dead_letter_topic))
}

/// The trailing segment of a fully-qualified resource path, for the two
/// columns that paint one. A value with no slash passes through unchanged,
/// which is what leaves `_deleted-topic_` intact.
fn short_id(resource_path: &str) -> String {
    resource_path
        .rsplit('/')
        .next()
        .unwrap_or(resource_path)
        .to_string()
}

fn compare_subscriptions(
    left: &PubSubSubscription,
    right: &PubSubSubscription,
    sort_field: PubSubSubscriptionSortField,
    direction: SortDirection,
) -> std::cmp::Ordering {
    let primary = match sort_field {
        PubSubSubscriptionSortField::Name => left.name.cmp(&right.name),
        // On the short id the column paints, not the resource path: sorting by
        // the full name would order by project prefix, which is identical for
        // every row and makes the sort a no-op.
        PubSubSubscriptionSortField::Topic => {
            parent_topic_label(left).cmp(&parent_topic_label(right))
        }
        PubSubSubscriptionSortField::Delivery => left.delivery.label().cmp(right.delivery.label()),
        PubSubSubscriptionSortField::State => left.state.label().cmp(&right.state.label()),
        // Numerically, not by the rendered label: `100` sorts after `60` in the
        // column an operator sorted to find the slowest ack, which text
        // ordering gets backwards. An absent deadline sorts as absent rather
        // than as zero, so the unknowns group at one end instead of claiming to
        // be the fastest.
        PubSubSubscriptionSortField::AckDeadline => {
            left.ack_deadline_seconds.cmp(&right.ack_deadline_seconds)
        }
        // RFC 3339 durations as the API returns them (`604800s`); an unset
        // retention is the empty string, so those rows group together rather
        // than scattering through the ordering.
        PubSubSubscriptionSortField::Retention => left
            .message_retention
            .as_deref()
            .unwrap_or_default()
            .cmp(right.message_retention.as_deref().unwrap_or_default()),
        // A subscription with no dead-letter policy sorts as the empty string,
        // grouping the rows that have none rather than interleaving them.
        PubSubSubscriptionSortField::DeadLetter => dead_letter_label(left)
            .unwrap_or_default()
            .cmp(&dead_letter_label(right).unwrap_or_default()),
    };
    // The name breaks every tie and is unique within the project, so this is
    // a total order over the row set.
    let ordering = primary.then_with(|| left.name.cmp(&right.name));
    match direction {
        SortDirection::Ascending => ordering,
        SortDirection::Descending => ordering.reverse(),
    }
}
