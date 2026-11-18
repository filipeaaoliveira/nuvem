//! Cloud Logging: the query, the entries it returns, and where a tail resumes.
//!
//! What a log surface needs and none of how it fetches. The filter expressions
//! are built here rather than at the call site so that quoting is decided in
//! exactly one place — see [`quote`] — and so the client-side severity floor
//! and the server-side `--freshness` window cannot come to mean different
//! things.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum LogSeverity {
    Default,
    Debug,
    Info,
    Notice,
    Warning,
    Error,
    Critical,
    Alert,
    Emergency,
}

impl LogSeverity {
    pub fn from_api(value: &str) -> Self {
        match value.to_ascii_uppercase().as_str() {
            "DEBUG" => Self::Debug,
            "INFO" => Self::Info,
            "NOTICE" => Self::Notice,
            "WARNING" => Self::Warning,
            "ERROR" => Self::Error,
            "CRITICAL" => Self::Critical,
            "ALERT" => Self::Alert,
            "EMERGENCY" => Self::Emergency,
            _ => Self::Default,
        }
    }

    /// The API's own spelling of the severity, which is also what a filter
    /// expression takes.
    pub fn label(self) -> &'static str {
        match self {
            Self::Default => "DEFAULT",
            Self::Debug => "DEBUG",
            Self::Info => "INFO",
            Self::Notice => "NOTICE",
            Self::Warning => "WARNING",
            Self::Error => "ERROR",
            Self::Critical => "CRITICAL",
            Self::Alert => "ALERT",
            Self::Emergency => "EMERGENCY",
        }
    }

    /// Position on the ladder, `Default` = 0 through `Emergency` = 8, for
    /// callers that need the number rather than the ordering the enum already
    /// has.
    pub fn rank(self) -> u8 {
        self as u8
    }

    /// Next severity up the ladder, saturating at `Emergency` so repeated
    /// key presses never wrap around to the no-floor sentinel.
    pub fn raise(self) -> LogSeverity {
        match self {
            Self::Default => Self::Debug,
            Self::Debug => Self::Info,
            Self::Info => Self::Notice,
            Self::Notice => Self::Warning,
            Self::Warning => Self::Error,
            Self::Error => Self::Critical,
            Self::Critical => Self::Alert,
            Self::Alert | Self::Emergency => Self::Emergency,
        }
    }

    /// Next severity down the ladder, saturating at `Default` (no floor).
    pub fn lower(self) -> LogSeverity {
        match self {
            Self::Default | Self::Debug => Self::Default,
            Self::Info => Self::Debug,
            Self::Notice => Self::Info,
            Self::Warning => Self::Notice,
            Self::Error => Self::Warning,
            Self::Critical => Self::Error,
            Self::Alert => Self::Critical,
            Self::Emergency => Self::Alert,
        }
    }

    /// Cloud Logging filter clause for this severity used as a floor, e.g.
    /// `severity>=ERROR`. `Default` is the sentinel for "no floor" and yields
    /// `None`, so the rendered filter stays byte-identical to an unfiltered
    /// query.
    pub fn filter_clause(self) -> Option<String> {
        match self {
            Self::Default => None,
            other => Some(format!("severity>={}", other.label())),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LogPayload {
    Text(String),
    /// Pretty-printed JSON text (kept as a String so `nuvem-domain` never has
    /// to model a JSON value, i.e. no `serde_json` dependency — mirrors
    /// `ComputeInstanceDetail.raw_json`). The service layer serializes the
    /// parsed value into this string.
    Json(String),
    Empty,
}

/// One log entry, flattened to what a viewer paints plus what a tail needs in
/// order to resume.
///
/// `message` is the one-line form; `payload` is the same entry's body in full.
/// `insert_id` is what makes an entry identifiable, since timestamps are not
/// unique — see [`LogCursor`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogEntry {
    pub insert_id: String,
    pub timestamp: String,
    pub severity: LogSeverity,
    pub resource_type: String,
    pub resource_labels: BTreeMap<String, String>,
    pub log_name: String,
    pub message: String,
    pub payload: LogPayload,
    pub labels: BTreeMap<String, String>,
}

/// Since-time buckets, declared shortest first. The declaration order is the
/// `next()` cycle order and matches the `0`..=`6` bucket digits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LogTimeRange {
    OneMinute,
    FiveMinutes,
    FifteenMinutes,
    ThirtyMinutes,
    OneHour,
    SixHours,
    TwentyFourHours,
}

impl LogTimeRange {
    pub const DEFAULT: LogTimeRange = LogTimeRange::OneHour;

    /// The same window in minutes.
    ///
    /// `--freshness` is a gcloud flag with no API equivalent: a native read
    /// expresses the window as a `timestamp>=` clause instead, and needs the
    /// bucket as a duration to build one. Kept beside `freshness_arg` so the
    /// two cannot describe different windows.
    pub fn minutes(self) -> i64 {
        match self {
            Self::OneMinute => 1,
            Self::FiveMinutes => 5,
            Self::FifteenMinutes => 15,
            Self::ThirtyMinutes => 30,
            Self::OneHour => 60,
            Self::SixHours => 6 * 60,
            Self::TwentyFourHours => 24 * 60,
        }
    }

    /// The value `gcloud logging read --freshness` takes for this bucket.
    pub fn freshness_arg(self) -> &'static str {
        match self {
            Self::OneMinute => "1m",
            Self::FiveMinutes => "5m",
            Self::FifteenMinutes => "15m",
            Self::ThirtyMinutes => "30m",
            Self::OneHour => "1h",
            Self::SixHours => "6h",
            Self::TwentyFourHours => "24h",
        }
    }

    /// Display label; identical to the `gcloud --freshness` argument for every
    /// bucket, so the UI shows exactly what is sent to the CLI.
    pub fn label(self) -> &'static str {
        self.freshness_arg()
    }

    /// The next bucket up, wrapping from the longest back to the shortest so
    /// one key can cycle the whole ladder.
    pub fn next(self) -> LogTimeRange {
        match self {
            Self::OneMinute => Self::FiveMinutes,
            Self::FiveMinutes => Self::FifteenMinutes,
            Self::FifteenMinutes => Self::ThirtyMinutes,
            Self::ThirtyMinutes => Self::OneHour,
            Self::OneHour => Self::SixHours,
            Self::SixHours => Self::TwentyFourHours,
            Self::TwentyFourHours => Self::OneMinute,
        }
    }

    /// Direct bucket addressing: `'0'`..=`'6'` select the buckets shortest to
    /// longest. Any other character (including non-ASCII digits) yields `None`
    /// so callers can pass raw key presses through unmodified.
    pub fn from_bucket_digit(digit: char) -> Option<LogTimeRange> {
        match digit {
            '0' => Some(Self::OneMinute),
            '1' => Some(Self::FiveMinutes),
            '2' => Some(Self::FifteenMinutes),
            '3' => Some(Self::ThirtyMinutes),
            '4' => Some(Self::OneHour),
            '5' => Some(Self::SixHours),
            '6' => Some(Self::TwentyFourHours),
            _ => None,
        }
    }

    /// Inverse of [`LogTimeRange::from_bucket_digit`].
    pub fn bucket_digit(self) -> char {
        match self {
            Self::OneMinute => '0',
            Self::FiveMinutes => '1',
            Self::FifteenMinutes => '2',
            Self::ThirtyMinutes => '3',
            Self::OneHour => '4',
            Self::SixHours => '5',
            Self::TwentyFourHours => '6',
        }
    }
}

/// Where a tail resumes: the timestamp of the newest entry seen, and every
/// insert id seen at exactly that timestamp.
///
/// The id list is what keeps a resume from either duplicating or skipping.
/// Cloud Logging timestamps are not unique, so a bound of `> timestamp` drops
/// the entries sharing the newest instant while `>=` re-delivers them; the ids
/// are how the ones already seen are recognised.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogCursor {
    pub timestamp: String,
    pub insert_ids: Vec<String>,
}

/// Removes control characters from untrusted log text so nothing can move the
/// cursor, clear the screen, or emit escape sequences. Tabs become spaces;
/// newlines are dropped to keep the compact one-line message form. Other C0/C1
/// control characters (including ESC) are removed entirely.
pub fn sanitize_log_text(value: &str) -> String {
    value
        .chars()
        .filter_map(|ch| match ch {
            '\t' => Some(' '),
            '\n' | '\r' => None,
            c if c.is_control() => None,
            c => Some(c),
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LogResourceKind {
    ComputeInstance {
        instance_id: String,
        instance_name: String,
        zone: String,
    },
    GkeCluster {
        cluster_name: String,
        location: String,
        project: String,
    },
    CloudRunService {
        service_name: String,
        region: String,
        project: String,
    },
    CloudSqlInstance {
        instance_name: String,
        project: String,
    },
    StorageBucket {
        bucket_name: String,
    },
    /// A Pub/Sub topic.
    ///
    /// Keyed by `topic_id`, the short id, rather than the resource path:
    /// `pubsub_topic` entries carry the short form in that label, and the
    /// project is already the query's scope.
    PubSubTopic {
        topic_name: String,
    },
    /// A Pub/Sub subscription.
    ///
    /// Keyed by `subscription_id`, the short id, for the same reason a topic is
    /// keyed by `topic_id`: `pubsub_subscription` entries carry the short form
    /// in that label, and the project is already the query's scope.
    ///
    /// Its own resource type rather than a filter on the topic's: a
    /// subscription's delivery failures are logged against the subscription,
    /// and a topic's entries do not include them.
    PubSubSubscription {
        subscription_name: String,
    },
    /// A VPC subnet.
    ///
    /// This is the one VPC resource that *does* produce logs, and it is why
    /// the networks table refuses `L` rather than offering it: Cloud Logging
    /// attributes both VPC flow logs and firewall hit logs to
    /// `gce_subnetwork`, never to the network and never to the firewall rule.
    /// An operator asking "what traffic is this carrying" is asking about a
    /// subnet whether they know it or not.
    ///
    /// Keyed by `subnetwork_name` — the id, not the self-link — because that
    /// is what the label carries and what the table paints. The region is not
    /// part of the query: two subnets in different regions can share a name,
    /// so this can over-match, and that is the safe direction. A query
    /// narrowed by a label the entries may not carry would return nothing and
    /// read as "no traffic", which is the dangerous one.
    VpcSubnet {
        subnet_name: String,
    },
    /// A Secret Manager secret. Its log entries are Cloud Audit Logs — who
    /// read or changed the secret's *metadata*, never the material itself.
    Secret {
        secret_name: String,
        project: String,
    },
    CloudRunRevision {
        service_name: String,
        revision_name: String,
        region: String,
        project: String,
    },
    /// A Cloud Function, of either generation.
    ///
    /// The generation is deliberately not a field: a 2nd gen function runs on
    /// Cloud Run and writes `cloud_run_revision` entries under its own name,
    /// while a 1st gen function writes `cloud_function` entries — and a
    /// function whose `environment` came back unspecified (an absent proto
    /// zero value, or a value nuvem does not recognise) would have to guess.
    /// The filter asks for both shapes instead of trusting the field.
    CloudFunction {
        function_name: String,
        region: String,
    },
    /// A BigQuery dataset.
    ///
    /// Keyed by `dataset_id`, the short id, the way a Pub/Sub topic is: the
    /// `bigquery_dataset` monitored resource carries the short form in that
    /// label, and the project is already the query's scope.
    ///
    /// What this returns is the dataset's audit trail — who read or changed
    /// the tables in it — because BigQuery writes no resource logs of its own.
    /// Query *jobs* are not in it: a job is logged under `bigquery_project`,
    /// keyed by project rather than by dataset, so folding it in would widen a
    /// dataset-scoped jump to every job in the project.
    ///
    /// Being an audit trail means a *correct* filter can still return nothing,
    /// which on screen reads as "this dataset has no logs" — the deferral's
    /// original worry arriving by another route. Two ways it happens: nothing
    /// touched the dataset inside the query's freshness window, or the project
    /// does not write the entries. BigQuery is the one product the "Data
    /// Access audit logs are off by default" rule exempts, so the second is a
    /// configuration to check — an exclusion filter on the `_Default` sink
    /// drops the entries before they are stored — rather than the norm. The hazard is not this variant's alone: every arm
    /// that queries an audit trail carries it, and `Secret` carries it harder,
    /// since Secret Manager has no such exemption.
    BigQueryDataset {
        dataset_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LogQuery {
    ProjectLogs,
    Resource(LogResourceKind),
}

/// Quotes a value for interpolation into a Cloud Logging filter expression.
/// Public because the service layer composes one clause of its own (the
/// tail-cursor's timestamp bound) and must not grow a second, subtly
/// different escaping.
pub fn quote(value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

impl LogQuery {
    /// The Cloud Logging filter expression that selects this query's entries.
    /// Empty for whole-project logs — the absence of a resource clause, not a
    /// filter that matches nothing.
    pub fn render_filter(&self) -> String {
        match self {
            LogQuery::ProjectLogs => String::new(),
            LogQuery::Resource(LogResourceKind::ComputeInstance { instance_id, .. }) => {
                format!(
                    "resource.type=\"gce_instance\" AND resource.labels.instance_id={}",
                    quote(instance_id)
                )
            }
            LogQuery::Resource(LogResourceKind::GkeCluster {
                cluster_name,
                location,
                ..
            }) => format!(
                "resource.type=\"k8s_cluster\" AND resource.labels.cluster_name={} AND resource.labels.location={}",
                quote(cluster_name),
                quote(location)
            ),
            LogQuery::Resource(LogResourceKind::CloudRunService {
                service_name,
                region,
                ..
            }) => format!(
                "resource.type=\"cloud_run_revision\" AND resource.labels.service_name={} AND resource.labels.location={}",
                quote(service_name),
                quote(region)
            ),
            // Cloud SQL log entries carry a single composite `database_id`
            // label of the form `<project>:<instance>`; there is no separate
            // instance label to filter on.
            LogQuery::Resource(LogResourceKind::CloudSqlInstance {
                instance_name,
                project,
            }) => format!(
                "resource.type=\"cloudsql_database\" AND resource.labels.database_id={}",
                quote(&format!("{project}:{instance_name}"))
            ),
            LogQuery::Resource(LogResourceKind::StorageBucket { bucket_name }) => format!(
                "resource.type=\"gcs_bucket\" AND resource.labels.bucket_name={}",
                quote(bucket_name)
            ),
            LogQuery::Resource(LogResourceKind::PubSubTopic { topic_name }) => format!(
                "resource.type=\"pubsub_topic\" AND resource.labels.topic_id={}",
                quote(topic_name)
            ),
            LogQuery::Resource(LogResourceKind::PubSubSubscription { subscription_name }) => {
                format!(
                    "resource.type=\"pubsub_subscription\" AND resource.labels.subscription_id={}",
                    quote(subscription_name)
                )
            }
            LogQuery::Resource(LogResourceKind::VpcSubnet { subnet_name }) => format!(
                "resource.type=\"gce_subnetwork\" AND resource.labels.subnetwork_name={}",
                quote(subnet_name)
            ),
            // Secret Manager writes no resource logs of its own; what exists is
            // the audit trail, keyed by the secret's full resource name. The
            // service label keeps another product's audited resource of the
            // same name out of the result.
            LogQuery::Resource(LogResourceKind::Secret {
                secret_name,
                project,
            }) => format!(
                "resource.type=\"audited_resource\" AND resource.labels.service=\"secretmanager.googleapis.com\" AND protoPayload.resourceName={}",
                quote(&format!("projects/{project}/secrets/{secret_name}"))
            ),
            LogQuery::Resource(LogResourceKind::CloudRunRevision {
                service_name,
                revision_name,
                region,
                ..
            }) => format!(
                "resource.type=\"cloud_run_revision\" AND resource.labels.service_name={} AND resource.labels.revision_name={} AND resource.labels.location={}",
                quote(service_name),
                quote(revision_name),
                quote(region)
            ),
            // Both generations in one filter, because the row does not
            // reliably say which it is (see [`LogResourceKind::CloudFunction`]).
            // The two halves are disjoint on `resource.type`, so a gen1
            // function cannot pick up a same-named Cloud Run service's logs and
            // vice versa. The outer parentheses are load-bearing: this is the
            // one variant with a top-level `OR`, and every caller appends
            // AND-joined clauses (the severity floor, the incremental cursor)
            // to whatever this returns, so it has to be a single term rather
            // than rely on the filter language's operator precedence.
            LogQuery::Resource(LogResourceKind::CloudFunction {
                function_name,
                region,
            }) => format!(
                "((resource.type=\"cloud_function\" AND resource.labels.function_name={name} AND resource.labels.region={region}) OR (resource.type=\"cloud_run_revision\" AND resource.labels.service_name={name} AND resource.labels.location={region}))",
                name = quote(function_name),
                region = quote(region)
            ),
            LogQuery::Resource(LogResourceKind::BigQueryDataset { dataset_id }) => format!(
                "resource.type=\"bigquery_dataset\" AND resource.labels.dataset_id={}",
                quote(dataset_id)
            ),
        }
    }

    /// One line naming what is being tailed, for the viewer's header.
    pub fn summary(&self) -> String {
        match self {
            LogQuery::ProjectLogs => "project logs".to_string(),
            LogQuery::Resource(LogResourceKind::ComputeInstance {
                instance_name,
                zone,
                ..
            }) => {
                format!("gce_instance {instance_name} ({zone})")
            }
            LogQuery::Resource(LogResourceKind::GkeCluster {
                cluster_name,
                location,
                ..
            }) => {
                format!("k8s_cluster {cluster_name} ({location})")
            }
            LogQuery::Resource(LogResourceKind::CloudRunService {
                service_name,
                region,
                ..
            }) => {
                format!("cloud_run_revision {service_name} ({region})")
            }
            LogQuery::Resource(LogResourceKind::CloudSqlInstance {
                instance_name,
                project,
            }) => {
                format!("cloudsql_database {project}:{instance_name}")
            }
            LogQuery::Resource(LogResourceKind::StorageBucket { bucket_name }) => {
                format!("gcs_bucket {bucket_name}")
            }
            LogQuery::Resource(LogResourceKind::PubSubTopic { topic_name }) => {
                format!("pubsub_topic {topic_name}")
            }
            LogQuery::Resource(LogResourceKind::PubSubSubscription { subscription_name }) => {
                format!("pubsub_subscription {subscription_name}")
            }
            LogQuery::Resource(LogResourceKind::VpcSubnet { subnet_name }) => {
                format!("gce_subnetwork {subnet_name}")
            }
            LogQuery::Resource(LogResourceKind::Secret { secret_name, .. }) => {
                format!("secret {secret_name}")
            }
            LogQuery::Resource(LogResourceKind::CloudRunRevision {
                revision_name,
                region,
                ..
            }) => {
                format!("cloud_run_revision {revision_name} ({region})")
            }
            LogQuery::Resource(LogResourceKind::CloudFunction {
                function_name,
                region,
            }) => {
                format!("cloud_function {function_name} ({region})")
            }
            LogQuery::Resource(LogResourceKind::BigQueryDataset { dataset_id }) => {
                format!("bigquery_dataset {dataset_id}")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severity_raise_and_lower_clamp_at_ladder_ends() {
        assert_eq!(LogSeverity::Warning.raise(), LogSeverity::Error);
        assert_eq!(LogSeverity::Error.lower(), LogSeverity::Warning);
        assert_eq!(LogSeverity::Emergency.raise(), LogSeverity::Emergency);
        assert_eq!(LogSeverity::Default.lower(), LogSeverity::Default);
        assert_eq!(LogSeverity::Default.raise(), LogSeverity::Debug);
        assert_eq!(LogSeverity::Emergency.lower(), LogSeverity::Alert);
    }

    #[test]
    fn severity_filter_clause_is_none_for_default_floor() {
        assert_eq!(LogSeverity::Default.filter_clause(), None);
        assert_eq!(
            LogSeverity::Error.filter_clause(),
            Some("severity>=ERROR".to_string())
        );
        assert_eq!(
            LogSeverity::Emergency.filter_clause(),
            Some("severity>=EMERGENCY".to_string())
        );
    }

    #[test]
    fn time_range_bucket_digits_round_trip() {
        let expected = [
            ('0', LogTimeRange::OneMinute, "1m"),
            ('1', LogTimeRange::FiveMinutes, "5m"),
            ('2', LogTimeRange::FifteenMinutes, "15m"),
            ('3', LogTimeRange::ThirtyMinutes, "30m"),
            ('4', LogTimeRange::OneHour, "1h"),
            ('5', LogTimeRange::SixHours, "6h"),
            ('6', LogTimeRange::TwentyFourHours, "24h"),
        ];
        for (digit, range, text) in expected {
            assert_eq!(LogTimeRange::from_bucket_digit(digit), Some(range));
            assert_eq!(range.bucket_digit(), digit);
            assert_eq!(range.freshness_arg(), text);
            assert_eq!(range.label(), text);
        }
    }

    #[test]
    fn time_range_rejects_non_bucket_digits() {
        assert_eq!(LogTimeRange::from_bucket_digit('7'), None);
        assert_eq!(LogTimeRange::from_bucket_digit('a'), None);
        assert_eq!(LogTimeRange::from_bucket_digit('٣'), None);
    }

    #[test]
    fn time_range_next_cycles_ascending_and_wraps() {
        assert_eq!(LogTimeRange::DEFAULT, LogTimeRange::OneHour);
        assert_eq!(LogTimeRange::OneMinute.next(), LogTimeRange::FiveMinutes);
        assert_eq!(
            LogTimeRange::FiveMinutes.next(),
            LogTimeRange::FifteenMinutes
        );
        assert_eq!(
            LogTimeRange::FifteenMinutes.next(),
            LogTimeRange::ThirtyMinutes
        );
        assert_eq!(LogTimeRange::ThirtyMinutes.next(), LogTimeRange::OneHour);
        assert_eq!(LogTimeRange::OneHour.next(), LogTimeRange::SixHours);
        assert_eq!(LogTimeRange::SixHours.next(), LogTimeRange::TwentyFourHours);
        assert_eq!(
            LogTimeRange::TwentyFourHours.next(),
            LogTimeRange::OneMinute
        );
    }

    #[test]
    fn cloud_run_service_renders_resource_scoped_filter() {
        let query = LogQuery::Resource(LogResourceKind::CloudRunService {
            service_name: "orders-api".to_string(),
            region: "europe-west1".to_string(),
            project: "dev-project".to_string(),
        });
        assert_eq!(
            query.render_filter(),
            "resource.type=\"cloud_run_revision\" AND resource.labels.service_name=\"orders-api\" AND resource.labels.location=\"europe-west1\""
        );
        assert_eq!(
            query.summary(),
            "cloud_run_revision orders-api (europe-west1)"
        );
    }

    /// Both generations, in one filter.
    ///
    /// A 1st gen function logs under `cloud_function`; a 2nd gen one runs on
    /// Cloud Run and logs under `cloud_run_revision` with the function's name
    /// as the service name. The row's `environment` is not consulted — it can
    /// come back unspecified — so both halves are always asked for, and the
    /// `resource.type` in each is what keeps a same-named Cloud Run service's
    /// logs out of the result.
    #[test]
    fn cloud_function_renders_a_filter_covering_both_generations() {
        let query = LogQuery::Resource(LogResourceKind::CloudFunction {
            function_name: "thumbnailer".to_string(),
            region: "europe-west1".to_string(),
        });
        assert_eq!(
            query.render_filter(),
            "((resource.type=\"cloud_function\" AND resource.labels.function_name=\"thumbnailer\" AND resource.labels.region=\"europe-west1\") OR (resource.type=\"cloud_run_revision\" AND resource.labels.service_name=\"thumbnailer\" AND resource.labels.location=\"europe-west1\"))"
        );
        assert_eq!(query.summary(), "cloud_function thumbnailer (europe-west1)");
    }

    #[test]
    fn cloud_sql_instance_renders_resource_scoped_filter() {
        let query = LogQuery::Resource(LogResourceKind::CloudSqlInstance {
            instance_name: "orders-db".to_string(),
            project: "dev-project".to_string(),
        });
        assert_eq!(
            query.render_filter(),
            "resource.type=\"cloudsql_database\" AND resource.labels.database_id=\"dev-project:orders-db\""
        );
        assert_eq!(query.summary(), "cloudsql_database dev-project:orders-db");
    }

    #[test]
    fn storage_bucket_renders_resource_scoped_filter() {
        let query = LogQuery::Resource(LogResourceKind::StorageBucket {
            bucket_name: "assets-dev".to_string(),
        });
        assert_eq!(
            query.render_filter(),
            "resource.type=\"gcs_bucket\" AND resource.labels.bucket_name=\"assets-dev\""
        );
        assert_eq!(query.summary(), "gcs_bucket assets-dev");
    }

    #[test]
    fn cloud_run_revision_renders_resource_scoped_filter() {
        let query = LogQuery::Resource(LogResourceKind::CloudRunRevision {
            service_name: "orders-api".to_string(),
            revision_name: "orders-api-00007-abc".to_string(),
            region: "europe-west1".to_string(),
            project: "dev-project".to_string(),
        });
        assert_eq!(
            query.render_filter(),
            "resource.type=\"cloud_run_revision\" AND resource.labels.service_name=\"orders-api\" AND resource.labels.revision_name=\"orders-api-00007-abc\" AND resource.labels.location=\"europe-west1\""
        );
        assert_eq!(
            query.summary(),
            "cloud_run_revision orders-api-00007-abc (europe-west1)"
        );
    }

    /// The dataset id is the whole scope, and it is quoted.
    ///
    /// The `resource.type` half is what keeps another product's `dataset_id`
    /// label out of the result, and the project half of the scope comes from
    /// the query's project rather than from a `project_id` clause — the same
    /// shape the Pub/Sub arms use.
    ///
    /// A plain id is the only case there is: a BigQuery dataset id admits
    /// letters, digits and underscores and nothing else, so neither character
    /// `quote` escapes can occur in one. It is quoted anyway, like every other
    /// arm's value, rather than interpolated bare.
    #[test]
    fn bigquery_dataset_renders_resource_scoped_filter() {
        let query = LogQuery::Resource(LogResourceKind::BigQueryDataset {
            dataset_id: "analytics_events".to_string(),
        });
        assert_eq!(
            query.render_filter(),
            "resource.type=\"bigquery_dataset\" AND resource.labels.dataset_id=\"analytics_events\""
        );
        assert_eq!(query.summary(), "bigquery_dataset analytics_events");
    }

    #[test]
    fn bigquery_dataset_kind_round_trips_serde() {
        let kind = LogResourceKind::BigQueryDataset {
            dataset_id: "analytics_events".to_string(),
        };
        let json = serde_json::to_string(&kind).unwrap();
        assert_eq!(
            serde_json::from_str::<LogResourceKind>(&json).unwrap(),
            kind
        );
    }

    #[test]
    fn cloud_sql_instance_kind_round_trips_serde() {
        let kind = LogResourceKind::CloudSqlInstance {
            instance_name: "orders-db".to_string(),
            project: "dev-project".to_string(),
        };
        let json = serde_json::to_string(&kind).unwrap();
        assert_eq!(
            serde_json::from_str::<LogResourceKind>(&json).unwrap(),
            kind
        );
    }

    #[test]
    fn storage_bucket_kind_round_trips_serde() {
        let kind = LogResourceKind::StorageBucket {
            bucket_name: "assets-dev".to_string(),
        };
        let json = serde_json::to_string(&kind).unwrap();
        assert_eq!(
            serde_json::from_str::<LogResourceKind>(&json).unwrap(),
            kind
        );
    }

    #[test]
    fn cloud_run_revision_kind_round_trips_serde() {
        let kind = LogResourceKind::CloudRunRevision {
            service_name: "orders-api".to_string(),
            revision_name: "orders-api-00007-abc".to_string(),
            region: "europe-west1".to_string(),
            project: "dev-project".to_string(),
        };
        let json = serde_json::to_string(&kind).unwrap();
        assert_eq!(
            serde_json::from_str::<LogResourceKind>(&json).unwrap(),
            kind
        );
    }
}
