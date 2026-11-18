//! Collector for the non-fatal problems found while resolving the config.
//!
//! Resolution never fails outright: a bad file, an unknown enum value or an
//! invalid `[columns]` block degrades to a problem and a defaulted field.
//! Every validator pushes into the one [`ConfigProblems`] so callers can
//! surface *all* of them rather than only the first.

/// Which part of the config produced a problem: the tag a reader uses to say
/// where a message came from, and the seam another validator (jump
/// rules) extends. The declaration order carries no meaning: problems are
/// reported in the insertion order [`ConfigProblems`] preserves, which is why
/// a `Load` problem is pushed last despite being declared first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ConfigProblemSource {
    /// The config file itself could not be read, parsed, or declared an
    /// unsupported version.
    Load,
    /// A field under `[general]`.
    General,
    /// An entry under `[aliases]`.
    Aliases,
    /// An entry under `[hotkeys]`.
    Hotkeys,
    /// A `[columns.<service>]` block.
    Columns,
    /// A `[[jumps]]` rule.
    Jumps,
    /// A `[[plugins]]` entry in the separate plugin file. Tagged
    /// apart from `Load` because the plugin file failing is not the config
    /// file failing: the user's theme, columns and jumps are unaffected.
    Plugins,
    /// An entry under `[favorites]`.
    Favorites,
    /// The persisted state file (see [`crate::config::state`]), which the app
    /// writes for itself rather than the user writing by hand. Tagged
    /// separately from `Load` so a warning about state the app can rebuild
    /// from defaults is not read as a problem with the user's config.
    State,
    /// An entry under `[theme]` — including the deferred check that runs once
    /// the gcloud configurations are known and an override names none of them.
    Theme,
}

/// One non-fatal config problem, tagged with the validator that found it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigProblem {
    pub source: ConfigProblemSource,
    pub message: String,
}

/// The problems found while resolving one config, in report order.
///
/// Report order is **insertion order**, and that is deterministic without
/// sorting: `apply_file` calls the validators in a fixed order, and each of
/// them walks a `BTreeMap`, so the same file always yields the same sequence.
/// Do not sort — sorting would only scramble the within-source order the
/// validators deliberately produce.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ConfigProblems {
    items: Vec<ConfigProblem>,
}

impl ConfigProblems {
    /// Record one problem from `source`.
    pub fn push(&mut self, source: ConfigProblemSource, message: impl Into<String>) {
        self.items.push(ConfigProblem {
            source,
            message: message.into(),
        });
    }

    /// Record several problems from the same source, preserving their order.
    pub fn extend(
        &mut self,
        source: ConfigProblemSource,
        messages: impl IntoIterator<Item = String>,
    ) {
        for message in messages {
            self.push(source, message);
        }
    }

    /// The problems, in report order.
    pub fn iter(&self) -> impl Iterator<Item = &ConfigProblem> {
        self.items.iter()
    }

    /// Just the messages, in report order.
    pub fn messages(&self) -> Vec<&str> {
        self.items
            .iter()
            .map(|problem| problem.message.as_str())
            .collect()
    }

    /// Consume the collector and yield the owned messages in report order.
    pub fn into_messages(self) -> Vec<String> {
        self.items
            .into_iter()
            .map(|problem| problem.message)
            .collect()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_collector_reports_no_problems() {
        let problems = ConfigProblems::default();
        assert!(problems.is_empty());
        assert_eq!(problems.len(), 0);
        assert!(problems.messages().is_empty());
        assert_eq!(problems.iter().count(), 0);
    }

    #[test]
    fn insertion_order_is_preserved_across_sources() {
        let mut problems = ConfigProblems::default();
        problems.push(ConfigProblemSource::Columns, "third");
        problems.push(ConfigProblemSource::General, "first");
        problems.extend(
            ConfigProblemSource::Aliases,
            ["second".to_string(), "fourth".to_string()],
        );

        assert_eq!(problems.len(), 4);
        assert!(!problems.is_empty());
        assert_eq!(
            problems.messages(),
            vec!["third", "first", "second", "fourth"]
        );
        assert_eq!(
            problems
                .iter()
                .map(|problem| problem.source)
                .collect::<Vec<_>>(),
            vec![
                ConfigProblemSource::Columns,
                ConfigProblemSource::General,
                ConfigProblemSource::Aliases,
                ConfigProblemSource::Aliases,
            ]
        );
    }

    #[test]
    fn messages_match_iter_order() {
        let mut problems = ConfigProblems::default();
        problems.push(ConfigProblemSource::Load, "load failed");
        problems.push(ConfigProblemSource::Hotkeys, "bad hotkey");

        let from_iter: Vec<&str> = problems
            .iter()
            .map(|problem| problem.message.as_str())
            .collect();
        assert_eq!(problems.messages(), from_iter);
    }

    #[test]
    fn into_messages_yields_owned_messages_in_report_order() {
        let mut problems = ConfigProblems::default();
        problems.push(ConfigProblemSource::General, "a");
        problems.push(ConfigProblemSource::Columns, "b");

        assert_eq!(
            problems.into_messages(),
            vec!["a".to_string(), "b".to_string()]
        );
    }
}
